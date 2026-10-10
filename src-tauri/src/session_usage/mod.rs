//! Read-only native usage analysis. No source writes, persistent cache or billing claims.
mod cache;
mod claude;
mod codex;

use crate::error::{ensure_not_cancelled, AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, Metadata},
    io::{BufRead, BufReader, Read},
    path::{Component, Path},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_1h: u64,
    pub reasoning: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: Tokens,
}
#[derive(Debug, Clone, Default)]
pub struct ParsedUsage {
    pub models: Vec<ModelUsage>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsageSource {
    pub provider: String,
    pub rollout_path: String,
    pub id: String,
    pub title: String,
    pub cwd: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct SessionUsage {
    #[serde(flatten)]
    pub source: UsageSource,
    pub models: Vec<ModelUsage>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct UsageFailure {
    pub path: String,
    pub error: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct UsageStatus {
    pub state: String,
    pub total_files: usize,
    pub processed_files: usize,
    pub cache_hits: usize,
    pub parsed_files: usize,
    pub elapsed_ms: u64,
    pub results: Vec<SessionUsage>,
    pub failures: Vec<UsageFailure>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct UsageStarted {
    pub job_id: u64,
}
struct Job {
    cancel: AtomicBool,
    status: Mutex<UsageStatus>,
    started: std::time::Instant,
}
static JOBS: OnceLock<Mutex<BTreeMap<u64, Arc<Job>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_LINE: u64 = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 250_000;

fn error(message: impl Into<String>) -> AppError {
    AppError::Other(message.into())
}
fn jobs() -> &'static Mutex<BTreeMap<u64, Arc<Job>>> {
    JOBS.get_or_init(Default::default)
}
fn job(id: u64) -> AppResult<Arc<Job>> {
    jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or_else(|| error("用量分析任务不存在或已过期"))
}
pub fn status(id: u64) -> AppResult<UsageStatus> {
    let task = job(id)?;
    let mut state = task
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if state.state == "running" {
        state.elapsed_ms = elapsed(&task);
    }
    Ok(state)
}
fn elapsed(task: &Job) -> u64 {
    task.started.elapsed().as_millis().min(u64::MAX as u128) as u64
}
pub fn cancel(id: u64) -> AppResult<()> {
    job(id)?.cancel.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
pub fn start(
    sessions: Vec<UsageSource>,
    codex_dir: String,
    claude_dir: String,
) -> AppResult<UsageStarted> {
    start_with_options(sessions, codex_dir, claude_dir, false)
}
pub fn start_with_options(
    sessions: Vec<UsageSource>,
    codex_dir: String,
    claude_dir: String,
    force_refresh: bool,
) -> AppResult<UsageStarted> {
    if sessions.len() > 10_000 {
        return Err(error("单次用量分析最多支持 10000 个会话，请缩小筛选范围"));
    }
    let mut seen = HashSet::new();
    let sessions: Vec<_> = sessions
        .into_iter()
        .filter(|s| seen.insert((s.provider.clone(), s.rollout_path.clone())))
        .collect();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let task = Arc::new(Job {
        cancel: AtomicBool::new(false),
        started: std::time::Instant::now(),
        status: Mutex::new(UsageStatus {
            state: "running".into(),
            total_files: sessions.len(),
            processed_files: 0,
            cache_hits: 0,
            parsed_files: 0,
            elapsed_ms: 0,
            results: vec![],
            failures: vec![],
            error: None,
        }),
    });
    {
        let mut all = jobs().lock().unwrap_or_else(|e| e.into_inner());
        if all
            .values()
            .filter(|j| j.status.lock().unwrap_or_else(|e| e.into_inner()).state == "running")
            .count()
            >= 2
        {
            return Err(error("已有两个用量分析任务正在运行，请先取消或等待完成"));
        }
        while all.len() >= 8 {
            let old = all
                .iter()
                .find(|(_, j)| {
                    j.status.lock().unwrap_or_else(|e| e.into_inner()).state != "running"
                })
                .map(|(id, _)| *id);
            if let Some(old) = old {
                all.remove(&old);
            } else {
                break;
            }
        }
        all.insert(id, task.clone());
    }
    let spawned = std::thread::Builder::new()
        .name("session-usage".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for source in sessions {
                    if task.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let root = match source.provider.as_str() {
                        "codex" => &codex_dir,
                        "claude" => &claude_dir,
                        _ => "",
                    };
                    let result = read_usage_cached(
                        &source,
                        Path::new(root),
                        &task.cancel,
                        force_refresh,
                        cache::shared(),
                        || {},
                    );
                    let mut state = task.status.lock().unwrap_or_else(|e| e.into_inner());
                    if matches!(result, Err(AppError::Cancelled)) {
                        break;
                    }
                    state.processed_files += 1;
                    match result {
                        Ok((parsed, hit)) => {
                            if hit {
                                state.cache_hits += 1;
                            } else {
                                state.parsed_files += 1;
                            }
                            state.results.push(SessionUsage {
                                source,
                                models: parsed.models,
                                warnings: parsed.warnings,
                            });
                        }
                        Err(e) => state.failures.push(UsageFailure {
                            path: source.rollout_path,
                            error: e.to_string(),
                        }),
                    }
                }
            }));
            let mut state = task.status.lock().unwrap_or_else(|e| e.into_inner());
            state.elapsed_ms = elapsed(&task);
            state.state = if result.is_err() {
                state.error = Some("用量分析线程异常，已保留完成结果".into());
                "failed"
            } else if task.cancel.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "completed"
            }
            .into();
        });
    if let Err(e) = spawned {
        jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        return Err(e.into());
    }
    Ok(UsageStarted { job_id: id })
}

fn validate(source: &UsageSource, root: &Path) -> AppResult<()> {
    if !root.is_absolute() {
        return Err(error("用量分析需要已配置的绝对来源路径"));
    }
    let path = Path::new(&source.rollout_path);
    let relative = path
        .strip_prefix(root)
        .map_err(|_| error("会话不在当前来源内"))?;
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        || path.extension().and_then(|s| s.to_str()) != Some("jsonl")
    {
        return Err(error("用量分析仅支持来源内的 JSONL 会话"));
    }
    let first = relative
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str());
    let layout = match source.provider.as_str() {
        "codex" => matches!(first, Some("sessions" | "archived_sessions")),
        "claude" => first == Some("projects") && relative.components().count() >= 3,
        _ => false,
    };
    if !layout {
        return Err(error("会话路径不符合 Codex/Claude 原生布局"));
    }
    crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "用量来源",
    )?;
    Ok(())
}
fn fingerprint(metadata: &Metadata) -> AppResult<(u64, std::time::SystemTime, String)> {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    };
    #[cfg(windows)]
    let identity = {
        use std::os::windows::fs::MetadataExt;
        metadata.creation_time().to_string()
    };
    #[cfg(not(any(unix, windows)))]
    let identity = format!("{:?}", metadata.created()?);
    Ok((metadata.len(), metadata.modified()?, identity))
}
#[cfg(test)]
fn read_usage(source: &UsageSource, root: &Path, cancel: &AtomicBool) -> AppResult<ParsedUsage> {
    read_usage_checked(source, root, cancel, || {})
}
#[cfg(test)]
fn read_usage_checked(
    source: &UsageSource,
    root: &Path,
    cancel: &AtomicBool,
    after_read: impl FnOnce(),
) -> AppResult<ParsedUsage> {
    read_usage_cached(source, root, cancel, true, cache::shared(), after_read)
        .map(|(parsed, _)| parsed)
}
fn read_usage_cached(
    source: &UsageSource,
    root: &Path,
    cancel: &AtomicBool,
    force_refresh: bool,
    cache: &Mutex<cache::UsageCache>,
    after_read: impl FnOnce(),
) -> AppResult<(ParsedUsage, bool)> {
    ensure_not_cancelled(Some(cancel))?;
    validate(source, root)?;
    let file = File::open(&source.rollout_path)?;
    let before = fingerprint(&file.metadata()?)?;
    if before.0 > MAX_FILE {
        return Err(error("会话超过 256 MiB 用量分析上限"));
    }
    validate(source, root)?;
    if before != fingerprint(&fs::metadata(&source.rollout_path)?)? {
        return Err(error("会话在打开期间发生变化，请重新分析"));
    }
    let key = cache::Key::new(source, root);
    let cached = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key, &before, force_refresh);
    if let Some(parsed) = cached {
        after_read();
        verify_final(source, root, cancel, &file, &before)?;
        return Ok((parsed, true));
    }
    let mut reader = BufReader::new((&file).take(MAX_FILE + 1));
    let mut line = Vec::new();
    let mut total = 0u64;
    let mut number = 0;
    let mut ended = false;
    let lines = std::iter::from_fn(|| {
        if ended {
            return None;
        }
        let parsed = (|| -> AppResult<Option<serde_json::Value>> {
            loop {
                ensure_not_cancelled(Some(cancel))?;
                line.clear();
                let count = (&mut reader)
                    .take(MAX_LINE + 1)
                    .read_until(b'\n', &mut line)?;
                if count == 0 {
                    return Ok(None);
                }
                total += count as u64;
                number += 1;
                if count as u64 > MAX_LINE || total > MAX_FILE || number > MAX_RECORDS {
                    return Err(error("会话超过用量分析的行长、记录数或大小上限"));
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(|_| error(format!("第 {number} 行 JSON 无效或正在写入，请稍后重试")));
            }
        })();
        match parsed {
            Ok(Some(v)) => Some(Ok(v)),
            Ok(None) => {
                ended = true;
                None
            }
            Err(e) => {
                ended = true;
                Some(Err(e))
            }
        }
    });
    let mut parsed = match source.provider.as_str() {
        "codex" => codex::parse(lines)?,
        "claude" => claude::parse(lines)?,
        _ => return Err(error("不支持的用量来源")),
    };
    after_read();
    verify_final(source, root, cancel, &file, &before)?;
    if parsed.models.is_empty() && parsed.warnings.is_empty() {
        parsed
            .warnings
            .push("未找到可用的原生用量记录，不能视为零消耗".into());
    }
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, before, parsed.clone());
    Ok((parsed, false))
}
fn verify_final(
    source: &UsageSource,
    root: &Path,
    cancel: &AtomicBool,
    file: &File,
    before: &cache::Fingerprint,
) -> AppResult<()> {
    ensure_not_cancelled(Some(cancel))?;
    validate(source, root)?;
    if *before != fingerprint(&file.metadata()?)?
        || *before != fingerprint(&fs::metadata(&source.rollout_path)?)?
    {
        return Err(error("会话在分析期间发生变化，请重新分析"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> UsageSource {
        let path = root.join("projects/demo/sample.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, row("m1")).unwrap();
        UsageSource {
            provider: "claude".into(),
            rollout_path: path.to_string_lossy().into_owned(),
            id: "sample".into(),
            title: "Sample".into(),
            cwd: "/synthetic".into(),
        }
    }
    fn row(id: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"assistant","requestId":id,"message":{"id":id,"model":"synthetic","usage":{"input_tokens":10,"output_tokens":3}}})
        )
    }
    #[test]
    fn reads_are_repeatable_after_append_and_do_not_modify_sources() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let s = fixture(dir.path());
        let cancel = AtomicBool::new(false);
        let first = read_usage(&s, dir.path(), &cancel).unwrap();
        assert_eq!(first.models[0].tokens.input, 10);
        assert_eq!(
            first.models,
            read_usage(&s, dir.path(), &cancel).unwrap().models
        );
        let body = format!("{}{}{}", row("m1"), row("m2"), row("m1"));
        fs::write(&s.rollout_path, &body).unwrap();
        let next = read_usage(&s, dir.path(), &cancel).unwrap();
        assert_eq!(next.models[0].tokens.input, 20);
        assert_eq!(fs::read_to_string(&s.rollout_path).unwrap(), body);
    }
    #[test]
    fn missing_outside_layout_corruption_and_cancellation_fail_closed() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let mut s = fixture(dir.path());
        let cancel = AtomicBool::new(false);
        assert!(matches!(
            read_usage(&s, dir.path(), &AtomicBool::new(true)),
            Err(AppError::Cancelled)
        ));
        assert!(read_usage(&s, &dir.path().join("other"), &cancel).is_err());
        let outside = dir.path().join("sample.jsonl");
        fs::write(&outside, row("x")).unwrap();
        let old = s.rollout_path.clone();
        s.rollout_path = outside.to_string_lossy().into_owned();
        assert!(read_usage(&s, dir.path(), &cancel).is_err());
        s.rollout_path = old;
        fs::write(&s.rollout_path, format!("{}{{", row("m1"))).unwrap();
        assert!(read_usage(&s, dir.path(), &cancel).is_err());
        fs::remove_file(&s.rollout_path).unwrap();
        assert!(read_usage(&s, dir.path(), &cancel).is_err());
    }
    #[test]
    fn final_fingerprint_rejects_changes_without_returning_partial_totals() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let s = fixture(dir.path());
        let result = read_usage_checked(&s, dir.path(), &AtomicBool::new(false), || {
            fs::write(&s.rollout_path, format!("{}{}", row("m1"), row("m2"))).unwrap();
        });
        assert!(result.unwrap_err().to_string().contains("发生变化"));
    }
    #[test]
    fn cache_reuses_only_unchanged_files_and_force_refresh_reparses() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let s = fixture(dir.path());
        let cache = Mutex::new(cache::UsageCache::new(10, 10000));
        let cancel = AtomicBool::new(false);
        let read =
            |force| read_usage_cached(&s, dir.path(), &cancel, force, &cache, || {}).unwrap();
        assert!(!read(false).1);
        assert!(read(false).1);
        assert!(!read(true).1);
        assert!(read(false).1);
        fs::write(&s.rollout_path, format!("{}{}", row("m1"), row("m2"))).unwrap();
        let (parsed, hit) = read(false);
        assert!(!hit);
        assert_eq!(parsed.models[0].tokens.input, 20);
        fs::write(&s.rollout_path, row("m1")).unwrap();
        let (parsed, hit) = read(false);
        assert!(!hit);
        assert_eq!(parsed.models[0].tokens.input, 10);
        let replacement = dir.path().join("replacement");
        fs::write(
            &replacement,
            format!("{}{}{}", row("x"), row("y"), row("z")),
        )
        .unwrap();
        fs::remove_file(&s.rollout_path).unwrap();
        fs::rename(replacement, &s.rollout_path).unwrap();
        let (parsed, hit) = read(false);
        assert!(!hit);
        assert_eq!(parsed.models[0].tokens.input, 30);
    }
    #[test]
    fn warm_cache_never_masks_failures_or_final_mutations() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let s = fixture(dir.path());
        let cache = Mutex::new(cache::UsageCache::new(10, 10000));
        let cancel = AtomicBool::new(false);
        read_usage_cached(&s, dir.path(), &cancel, false, &cache, || {}).unwrap();
        assert!(matches!(
            read_usage_cached(&s, dir.path(), &AtomicBool::new(true), false, &cache, || {}),
            Err(AppError::Cancelled)
        ));
        assert!(
            read_usage_cached(&s, &dir.path().join("other"), &cancel, false, &cache, || {})
                .is_err()
        );
        let final_cancel = read_usage_cached(&s, dir.path(), &cancel, false, &cache, || {
            cancel.store(true, Ordering::Release);
        });
        assert!(matches!(final_cancel, Err(AppError::Cancelled)));
        cancel.store(false, Ordering::Release);
        let final_changed = read_usage_cached(&s, dir.path(), &cancel, false, &cache, || {
            fs::write(&s.rollout_path, format!("{}{}", row("m1"), row("m2"))).unwrap();
        });
        assert!(final_changed.unwrap_err().to_string().contains("发生变化"));
        fs::write(&s.rollout_path, "{").unwrap();
        assert!(read_usage_cached(&s, dir.path(), &cancel, false, &cache, || {}).is_err());
        fs::remove_file(&s.rollout_path).unwrap();
        assert!(read_usage_cached(&s, dir.path(), &cancel, false, &cache, || {}).is_err());
    }
    #[test]
    fn jobs_deduplicate_scope_and_isolate_file_failures() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let s = fixture(dir.path());
        let mut bad = s.clone();
        bad.rollout_path += ".missing";
        let started = start(
            vec![s.clone(), s, bad],
            String::new(),
            dir.path().to_string_lossy().into_owned(),
        )
        .unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let state = status(started.job_id).unwrap();
            if state.state != "running" {
                assert_eq!(state.state, "completed");
                assert_eq!(state.total_files, 2);
                assert_eq!(state.processed_files, 2);
                assert_eq!(state.results.len(), 1);
                assert_eq!(state.cache_hits + state.parsed_files, 1);
                assert_eq!(state.failures.len(), 1);
                let mut renamed = state.results[0].source.clone();
                renamed.title = "New title".into();
                renamed.cwd = "/new-project".into();
                let next = start(
                    vec![renamed],
                    String::new(),
                    dir.path().to_string_lossy().into_owned(),
                )
                .unwrap();
                loop {
                    let warm = status(next.job_id).unwrap();
                    if warm.state != "running" {
                        assert_eq!(warm.cache_hits, 1);
                        assert_eq!(warm.parsed_files, 0);
                        assert_eq!(warm.results[0].source.title, "New title");
                        assert_eq!(warm.results[0].source.cwd, "/new-project");
                        break;
                    }
                    assert!(std::time::Instant::now() < until);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                break;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
