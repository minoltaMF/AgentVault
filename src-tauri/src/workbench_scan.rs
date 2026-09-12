use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{ensure_not_cancelled, AppError, AppResult};
use crate::models::SessionSummary;
use serde::Serialize;

const MAX_JOBS: usize = 16;
const MAX_RUNNING: usize = 4;
const MAX_ERRORS: usize = 200;

#[derive(Clone, Serialize)]
pub struct ScanFailure {
    pub path: String,
    pub message: String,
}
#[derive(Clone, Serialize)]
pub struct ScanStatus {
    pub job_id: u64,
    pub state: String,
    pub phase: String,
    pub discovered_files: usize,
    pub processed_files: usize,
    pub failed_files: usize,
    pub current_path: Option<String>,
    pub errors: Vec<ScanFailure>,
    pub errors_truncated: bool,
    pub error: Option<String>,
    pub results: Vec<SessionSummary>,
}
#[derive(Serialize)]
pub struct ScanStarted {
    pub job_id: u64,
}
struct Job {
    scope: String,
    cancel: AtomicBool,
    status: Mutex<ScanStatus>,
}
static JOBS: OnceLock<Mutex<BTreeMap<u64, Arc<Job>>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn jobs() -> &'static Mutex<BTreeMap<u64, Arc<Job>>> {
    JOBS.get_or_init(Default::default)
}
impl Job {
    fn new(id: u64) -> Self {
        Self {
            scope: String::new(),
            cancel: AtomicBool::new(false),
            status: Mutex::new(ScanStatus {
                job_id: id,
                state: "running".into(),
                phase: "discovering".into(),
                discovered_files: 0,
                processed_files: 0,
                failed_files: 0,
                current_path: None,
                errors: vec![],
                errors_truncated: false,
                error: None,
                results: vec![],
            }),
        }
    }
    fn update(&self, f: impl FnOnce(&mut ScanStatus)) {
        f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }
    fn check(&self) -> AppResult<()> {
        ensure_not_cancelled(Some(&self.cancel))
    }
    fn current(&self, phase: &str, path: &Path) {
        self.update(|s| {
            s.phase = phase.into();
            s.current_path = Some(path.to_string_lossy().into_owned());
        });
    }
    fn failure(&self, path: &Path, error: impl std::fmt::Display) {
        self.update(|s| {
            s.failed_files += 1;
            if s.errors.len() < MAX_ERRORS {
                s.errors.push(ScanFailure {
                    path: path.to_string_lossy().into_owned(),
                    message: error.to_string(),
                });
            } else {
                s.errors_truncated = true;
            }
        });
    }
}
fn get_job(id: u64) -> AppResult<Arc<Job>> {
    jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("扫描任务不存在或已过期: {id}")))
}
pub fn start_workbench_scan(
    provider: String,
    codex_dir: String,
    claude_dir: String,
) -> AppResult<ScanStarted> {
    let root = match provider.as_str() {
        "codex" => PathBuf::from(&codex_dir),
        "claude" => PathBuf::from(claude_dir),
        _ => return Err(AppError::Other("扫描仅支持 Codex / Claude".into())),
    };
    if !root.is_absolute() {
        return Err(AppError::Path("扫描来源必须是绝对路径".into()));
    }
    if root
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(AppError::Path("扫描来源不能包含路径穿越".into()));
    }
    for ancestor in root.ancestors() {
        if crate::path_safety::metadata_is_link_or_reparse(&fs::symlink_metadata(ancestor)?) {
            return Err(AppError::Path("扫描来源不能经过链接或 junction".into()));
        }
    }
    let canonical = root.canonicalize()?;
    let scope = format!("{provider}:{}", canonical.to_string_lossy());
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut new_job = Job::new(id);
    new_job.scope = scope.clone();
    let job = Arc::new(new_job);
    {
        let mut all = jobs().lock().unwrap_or_else(|e| e.into_inner());
        // Reattach after an uncertain start response instead of scanning the same source twice.
        if let Some((id, _)) = all.iter().find(|(_, j)| {
            j.scope == scope
                && j.status.lock().unwrap_or_else(|e| e.into_inner()).state == "running"
        }) {
            return Ok(ScanStarted { job_id: *id });
        }
        let running = all
            .values()
            .filter(|j| j.status.lock().unwrap_or_else(|e| e.into_inner()).state == "running")
            .count();
        if running >= MAX_RUNNING {
            return Err(AppError::Other(
                "同时扫描任务已达上限，请取消或等待现有任务".into(),
            ));
        }
        while all.len() >= MAX_JOBS {
            let oldest = all
                .iter()
                .find(|(_, j)| {
                    j.status.lock().unwrap_or_else(|e| e.into_inner()).state != "running"
                })
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                all.remove(&oldest);
            } else {
                break;
            }
        }
        all.insert(id, job.clone());
    }
    let worker = job.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(format!("workbench-scan-{id}"))
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scan(&worker, &provider, &root)?;
                worker.check()?;
                worker.current("annotating", &PathBuf::from(&codex_dir));
                let mut results = worker
                    .status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .results
                    .clone();
                if Path::new(&codex_dir).is_absolute() {
                    crate::provenance::annotate_sessions(Path::new(&codex_dir), &mut results);
                }
                worker.update(|s| s.results = results);
                worker.check()
            }));
            worker.update(|s| {
                match outcome {
                    Ok(Ok(())) if !worker.cancel.load(Ordering::Relaxed) => {
                        s.state = "completed".into()
                    }
                    Ok(Err(AppError::Cancelled)) | Ok(Ok(())) => s.state = "cancelled".into(),
                    Ok(Err(error)) => {
                        s.state = "failed".into();
                        s.error = Some(error.to_string());
                    }
                    Err(_) => {
                        s.state = "failed".into();
                        s.error = Some("扫描线程异常结束".into());
                    }
                }
                s.phase = "finished".into();
                s.current_path = None;
            });
        })
    {
        jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        return Err(error.into());
    }
    Ok(ScanStarted { job_id: id })
}
pub fn workbench_scan_status(job_id: u64) -> AppResult<ScanStatus> {
    let job = get_job(job_id)?;
    let s = job.status.lock().unwrap_or_else(|e| e.into_inner());
    // Do not clone accumulated sessions for each running poll.
    Ok(ScanStatus {
        job_id: s.job_id,
        state: s.state.clone(),
        phase: s.phase.clone(),
        discovered_files: s.discovered_files,
        processed_files: s.processed_files,
        failed_files: s.failed_files,
        current_path: s.current_path.clone(),
        errors: s.errors.clone(),
        errors_truncated: s.errors_truncated,
        error: s.error.clone(),
        results: if s.state == "running" {
            vec![]
        } else {
            s.results.clone()
        },
    })
}
pub fn cancel_workbench_scan(job_id: u64) -> AppResult<()> {
    get_job(job_id)?.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

// Explicit stack: links/junctions are reported and never traversed, and every directory entry is cancellable.
fn discover(
    job: &Job,
    root: &Path,
    mut visit: impl FnMut(&Path) -> AppResult<()>,
) -> AppResult<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        job.check()?;
        job.current("discovering", &path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path == root => continue,
            Err(e) => {
                job.failure(&path, e);
                continue;
            }
        };
        if crate::path_safety::metadata_is_link_or_reparse(&metadata) {
            job.failure(&path, "拒绝扫描符号链接或 junction");
            continue;
        }
        if metadata.is_dir() {
            match fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries {
                        job.check()?;
                        match entry {
                            Ok(entry) => stack.push(entry.path()),
                            Err(e) => job.failure(&path, e),
                        }
                    }
                }
                Err(e) => job.failure(&path, e),
            }
        } else if metadata.is_file() && path.extension().and_then(|v| v.to_str()) == Some("jsonl") {
            visit(&path)?;
        }
    }
    Ok(())
}
fn validate_jsonl(job: &Job, path: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if crate::path_safety::metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(AppError::Path("会话不是普通文件".into()));
    }
    validate_lines(job, BufReader::new(File::open(path)?))
}
fn validate_lines(job: &Job, reader: impl BufRead) -> AppResult<()> {
    let mut nonempty = false;
    for (line_no, line) in reader.lines().enumerate() {
        job.check()?;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        nonempty = true;
        serde_json::from_str::<serde_json::Value>(&line)
            .map_err(|e| AppError::Other(format!("第 {} 行 JSON 无效: {e}", line_no + 1)))?;
    }
    if !nonempty {
        return Err(AppError::Other("会话文件为空".into()));
    }
    Ok(())
}
fn process(
    job: &Job,
    path: &Path,
    parse: impl FnOnce() -> AppResult<Option<SessionSummary>>,
) -> AppResult<()> {
    job.check()?;
    job.update(|s| s.discovered_files += 1);
    job.current("reading", path);
    let result = validate_jsonl(job, path).and_then(|_| parse());
    match result {
        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
        Err(e) => job.failure(path, e),
        Ok(Some(session)) => job.update(|s| s.results.push(session)),
        Ok(None) => {} // Known duplicate; callers report invalid metadata explicitly.
    }
    job.update(|s| s.processed_files += 1);
    Ok(())
}
fn checked_process(
    job: &Job,
    root: &Path,
    path: &Path,
    parse: impl FnOnce() -> AppResult<Option<SessionSummary>>,
) -> AppResult<()> {
    job.check()?;
    if let Err(error) = crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "扫描文件",
    ) {
        job.update(|s| {
            s.discovered_files += 1;
            s.processed_files += 1;
        });
        job.failure(path, error);
        return Ok(());
    }
    process(job, path, parse)
}
fn scan(job: &Job, provider: &str, root: &Path) -> AppResult<()> {
    job.check()?;
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() || crate::path_safety::metadata_is_link_or_reparse(&meta) {
        return Err(AppError::Path("来源不是普通目录".into()));
    }
    if provider == "claude" {
        crate::path_safety::validate_descendant(
            root,
            &root.join("projects"),
            crate::path_safety::EntryKind::Directory,
            false,
            "Claude projects",
        )?;
        discover(job, &root.join("projects"), |path| {
            checked_process(job, root, path, || {
                crate::claude_sessions::parse_session(path, Some(&job.cancel))?
                    .map(Some)
                    .ok_or_else(|| AppError::Other("未找到有效会话元数据".into()))
            })
        })?;
    } else {
        for name in [
            "state_5.sqlite",
            "state_5.sqlite-wal",
            "state_5.sqlite-shm",
            "session_index.jsonl",
        ] {
            crate::path_safety::validate_descendant(
                root,
                &root.join(name),
                crate::path_safety::EntryKind::File,
                name != "state_5.sqlite",
                "Codex 索引",
            )?;
        }
        job.current("reading_index", &crate::paths::state_db_path(root));
        let rows = crate::sessions::workbench_codex_rows(root, &job.cancel).map_err(|e| {
            AppError::Other(format!(
                "{}: {e}",
                crate::paths::state_db_path(root).display()
            ))
        })?;
        let known_names: HashSet<_> = rows
            .iter()
            .filter_map(|s| Path::new(&s.rollout_path).file_name().map(|v| v.to_owned()))
            .collect();
        let known_ids: HashSet<_> = rows.iter().map(|s| s.id.clone()).collect();
        for mut row in rows {
            let path = PathBuf::from(&row.rollout_path);
            checked_process(job, root, &path, || {
                row.rollout_bytes = fs::metadata(&path)?.len();
                if row.tokens_used <= 0 {
                    row.tokens_used =
                        crate::rollout::read_rollout_token_total_cancellable(&path, &job.cancel)?;
                }
                Ok(Some(row))
            })?;
        }
        discover(job, &root.join("archived_sessions"), |path| {
            if path.file_name().is_some_and(|n| known_names.contains(n)) {
                return Ok(());
            }
            checked_process(job, root, path, || {
                let Some(b) =
                    crate::repair::read_rollout_brief_cancellable(root, path, &job.cancel)?
                else {
                    return Err(AppError::Other("未找到有效会话元数据".into()));
                };
                if known_ids.contains(&b.id) {
                    return Ok(None);
                }
                let cwd = b.cwd.unwrap_or_default();
                Ok(Some(SessionSummary {
                    provider: "codex".into(),
                    resume_command: format!("codex resume {}", b.id),
                    id: b.id,
                    rollout_path: path.to_string_lossy().into_owned(),
                    cwd_display: crate::paths::basename_display(&cwd),
                    cwd,
                    title: b.first_user_message.chars().take(80).collect(),
                    first_user_message: b.first_user_message,
                    model: b.model,
                    reasoning_effort: b.reasoning_effort,
                    source: b.source,
                    agent_nickname: None,
                    agent_role: None,
                    conversion_origin: None,
                    tokens_used: b.tokens_used,
                    created_at: b.created_at_ms / 1000,
                    updated_at: b.updated_at_ms / 1000,
                    archived: true,
                    git_branch: None,
                    rollout_bytes: fs::metadata(path)?.len(),
                    logs_count: 0,
                    has_backup: false,
                }))
            })
        })?;
    }
    job.check()?;
    job.update(|s| {
        s.results
            .sort_by_key(|row| std::cmp::Reverse(row.updated_at.max(row.created_at)))
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workbench_scan_rejects_missing_projects_and_outside_file_without_parsing() {
        let root = fixture();
        fs::remove_dir(root.join("projects")).unwrap();
        assert!(scan(&Job::new(0), "claude", &root).is_err());
        let job = Job::new(0);
        checked_process(
            &job,
            &root,
            &root.parent().unwrap().join("outside.jsonl"),
            || panic!("outside parser must not run"),
        )
        .unwrap();
        let status = job.status.lock().unwrap();
        assert_eq!(status.failed_files, 1);
        assert_eq!(status.processed_files, 1);
        assert!(status.results.is_empty());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn workbench_scan_cancels_during_line_reads() {
        struct Reader<'a> {
            data: std::io::Cursor<Vec<u8>>,
            cancel: &'a AtomicBool,
            reads: usize,
        }
        impl std::io::Read for Reader<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                std::io::Read::read(&mut self.data, buf)
            }
        }
        impl BufRead for Reader<'_> {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                self.reads += 1;
                if self.reads == 2 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                self.data.fill_buf()
            }
            fn consume(&mut self, amount: usize) {
                self.data.consume(amount);
            }
        }
        let job = Job::new(0);
        let reader = Reader {
            data: std::io::Cursor::new(b"{}\n{}\n{}\n".to_vec()),
            cancel: &job.cancel,
            reads: 0,
        };
        assert!(matches!(
            validate_lines(&job, reader),
            Err(AppError::Cancelled)
        ));
    }

    #[test]
    fn workbench_scan_cancel_is_job_scoped() {
        let a = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let b = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let first = Arc::new(Job::new(a));
        let second = Arc::new(Job::new(b));
        {
            let mut all = jobs().lock().unwrap();
            all.insert(a, first.clone());
            all.insert(b, second.clone());
        }
        cancel_workbench_scan(a).unwrap();
        assert!(matches!(first.check(), Err(AppError::Cancelled)));
        assert!(second.check().is_ok());
        let mut all = jobs().lock().unwrap();
        all.remove(&a);
        all.remove(&b);
    }
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agentvault-workbench-scan-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("projects")).unwrap();
        path
    }
    #[test]
    fn workbench_scan_isolates_invalid_json_and_preserves_old_claude_parser() {
        let root = fixture();
        let good = root.join("projects/good.jsonl");
        let bad = root.join("projects/bad.jsonl");
        fs::write(&good,"{\"sessionId\":\"good\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n").unwrap();
        fs::write(&bad, "not-json\n").unwrap();
        let job = Job::new(0);
        scan(&job, "claude", &root).unwrap();
        let s = job.status.lock().unwrap();
        assert_eq!(s.processed_files, 2);
        assert_eq!(s.discovered_files, 2);
        assert_eq!(s.failed_files, 1);
        assert_eq!(s.results.len(), 1);
        assert_eq!(s.results[0].id, "good");
        assert!(s.errors[0].message.contains("第 1 行"));
        assert_eq!(Path::new(&s.errors[0].path), bad.as_path());
        assert!(crate::claude_sessions::parse_session(&bad, None).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn workbench_scan_cancel_inside_discovery_stops_following_files() {
        let root = fixture();
        for i in 0..5 {
            fs::write(root.join("projects").join(format!("{i}.jsonl")), "{}\n").unwrap();
        }
        let job = Job::new(0);
        let mut visited = 0;
        let result = discover(&job, &root.join("projects"), |_| {
            visited += 1;
            job.cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(visited, 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn workbench_scan_cancellation_does_not_count_unprocessed_file() {
        let root = fixture();
        let path = root.join("projects/one.jsonl");
        fs::write(&path, "{}\n").unwrap();
        let job = Job::new(0);
        let result = process(&job, &path, || {
            job.cancel.store(true, Ordering::Relaxed);
            job.check()?;
            Ok(None)
        });
        assert!(matches!(result, Err(AppError::Cancelled)));
        let s = job.status.lock().unwrap();
        assert_eq!(s.discovered_files, 1);
        assert_eq!(s.processed_files, 0);
        assert_eq!(s.failed_files, 0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn workbench_scan_bounds_errors_and_rejects_implicit_roots() {
        assert!(start_workbench_scan("claude".into(), "".into(), "".into()).is_err());
        assert!(start_workbench_scan("cursor".into(), "".into(), "".into()).is_err());
        let job = Job::new(0);
        for _ in 0..250 {
            job.failure(Path::new("bad"), "failure");
        }
        let s = job.status.lock().unwrap();
        assert_eq!(s.failed_files, 250);
        assert_eq!(s.errors.len(), MAX_ERRORS);
        assert!(s.errors_truncated);
    }
    #[test]
    fn workbench_scan_jobs_are_independent_and_terminal_results_are_available() {
        let root = fixture();
        fs::write(root.join("projects/a.jsonl"), "{\"sessionId\":\"a\"}\n").unwrap();
        let started = start_workbench_scan(
            "claude".into(),
            "".into(),
            root.to_string_lossy().into_owned(),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let status = workbench_scan_status(started.job_id).unwrap();
            if status.state != "running" {
                assert_eq!(status.state, "completed");
                assert_eq!(status.results.len(), 1);
                break;
            }
            assert!(status.results.is_empty());
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(cancel_workbench_scan(u64::MAX).is_err());
        assert_eq!(
            workbench_scan_status(started.job_id).unwrap().state,
            "completed"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
