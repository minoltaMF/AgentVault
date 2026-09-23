//! Explicit, in-process conversation content search.
//!
//! Data flow:
//! 1. The UI starts one job with an explicit provider and visibility scope.
//! 2. A single worker streams matching rollout files without creating an index.
//! 3. Existing preview classifiers decide which JSONL rows are real conversation messages.
//! 4. The UI polls a bounded status snapshot and may cancel the active job.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{
    ContentSearchMatch, ContentSearchResult, ContentSearchStart, ContentSearchStatus, ProviderDirs,
    SessionSummary,
};
use crate::{rollout, sessions};

const MAX_MATCHING_SESSIONS: usize = 100;
const MAX_MATCHES_PER_SESSION: usize = 3;
const MAX_QUERY_CHARS: usize = 256;
const PROGRESS_STEP_BYTES: u64 = 4 * 1024 * 1024;
const SNIPPET_BEFORE_CHARS: usize = 72;
const SNIPPET_AFTER_CHARS: usize = 160;

#[derive(Clone)]
struct SearchJob {
    id: u64,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<ContentSearchStatus>>,
}

struct SearchManager {
    next_id: AtomicU64,
    active: Mutex<Option<SearchJob>>,
}

struct SearchRequest {
    provider: String,
    dirs: ProviderDirs,
    query: String,
    rollout_paths: Vec<String>,
    scopes: Option<Vec<ContentSearchScope>>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContentSearchScope {
    pub provider: String,
    pub rollout_paths: Vec<String>,
}

struct FileScanOutcome {
    matches: Vec<ContentSearchMatch>,
    bytes_read: u64,
    cancelled: bool,
    missing: bool,
}

impl SearchManager {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            active: Mutex::new(None),
        }
    }

    fn start(&self, request: SearchRequest) -> AppResult<ContentSearchStart> {
        validate_request(&request)?;

        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(job) = active.as_ref() {
            let status = job.status.lock().unwrap_or_else(|error| error.into_inner());
            if status.state == "running" {
                return Err(AppError::Other(
                    "已有全文搜索正在运行，请先停止当前搜索".to_string(),
                ));
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let job = SearchJob {
            id,
            cancel: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(ContentSearchStatus {
                failures: Vec::new(),
                failed_files: 0,
                job_id: id,
                state: "running".to_string(),
                query: request.query.clone(),
                scanned_files: 0,
                total_files: 0,
                skipped_files: 0,
                scanned_bytes: 0,
                total_bytes: 0,
                results: Vec::new(),
                truncated: false,
                error: None,
            })),
        };
        let worker_job = job.clone();
        thread::Builder::new()
            .name("cc-sessions-content-search".to_string())
            .spawn(move || run_job(worker_job, request))
            .map_err(|error| AppError::Other(format!("无法启动全文搜索任务: {error}")))?;
        *active = Some(job);
        Ok(ContentSearchStart { job_id: id })
    }

    fn status(&self, job_id: u64) -> AppResult<ContentSearchStatus> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let job = active
            .as_ref()
            .filter(|job| job.id == job_id)
            .ok_or_else(|| AppError::NotFound(format!("全文搜索任务 {job_id}")))?;
        let snapshot = job
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Ok(snapshot)
    }

    fn active(&self) -> Option<ContentSearchStart> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let job = active.as_ref()?;
        let status = job.status.lock().unwrap_or_else(|error| error.into_inner());
        (status.state == "running").then_some(ContentSearchStart { job_id: job.id })
    }

    fn cancel(&self, job_id: u64) -> AppResult<()> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let job = active
            .as_ref()
            .filter(|job| job.id == job_id)
            .ok_or_else(|| AppError::NotFound(format!("全文搜索任务 {job_id}")))?;
        let status = job.status.lock().unwrap_or_else(|error| error.into_inner());
        if status.state == "running" {
            job.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
}

fn manager() -> &'static SearchManager {
    static MANAGER: OnceLock<SearchManager> = OnceLock::new();
    MANAGER.get_or_init(SearchManager::new)
}

pub fn start_content_search(
    provider: String,
    dirs: ProviderDirs,
    query: String,
    rollout_paths: Vec<String>,
) -> AppResult<ContentSearchStart> {
    manager().start(SearchRequest {
        provider,
        dirs,
        query: query.trim().to_string(),
        rollout_paths,
        scopes: None,
    })
}

pub fn start_workbench_content_search(
    dirs: ProviderDirs,
    query: String,
    scopes: Vec<ContentSearchScope>,
) -> AppResult<ContentSearchStart> {
    for scope in &scopes {
        let root = match scope.provider.as_str() {
            "codex" => Some(dirs.codex_dir.as_str()),
            "claude" => dirs.claude_dir.as_deref(),
            "qoder" => dirs.qoder_dir.as_deref(),
            "workbuddy" => dirs.workbuddy_dir.as_deref(),
            "grok" => dirs.grok_dir.as_deref(),
            "pi" => dirs.pi_dir.as_deref(),
            "dsh" => dirs.dsh_dir.as_deref(),
            "hermes" => dirs.hermes_dir.as_deref(),
            "zcode" => dirs.zcode_dir.as_deref(),
            "opencode" => dirs.opencode_dir.as_deref(),
            _ => return Err(AppError::Other("不支持的工作台搜索来源".into())),
        };
        if root.is_none_or(|root| root.trim().is_empty()) {
            return Err(AppError::Other("搜索来源目录不能为空".into()));
        }
    }
    manager().start(SearchRequest {
        provider: "workbench".into(),
        dirs,
        query: query.trim().into(),
        rollout_paths: Vec::new(),
        scopes: Some(scopes),
    })
}

pub fn content_search_status(job_id: u64) -> AppResult<ContentSearchStatus> {
    manager().status(job_id)
}

pub fn active_content_search() -> AppResult<Option<ContentSearchStart>> {
    Ok(manager().active())
}

pub fn cancel_content_search(job_id: u64) -> AppResult<()> {
    manager().cancel(job_id)
}

fn validate_request(request: &SearchRequest) -> AppResult<()> {
    if request.provider == "workbench" && request.scopes.is_none() {
        return Err(AppError::Other("工作台搜索必须提供明确来源范围".into()));
    }
    if !matches!(
        request.provider.as_str(),
        "codex" | "claude" | "opencode" | "cursor" | "workbench"
    ) {
        return Err(AppError::Other(format!(
            "不支持的 provider: {}",
            request.provider
        )));
    }
    if request.query.chars().count() < 2 {
        return Err(AppError::Other(
            "全文搜索关键词至少需要 2 个字符".to_string(),
        ));
    }
    if request.query.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::Other(format!(
            "全文搜索关键词不能超过 {MAX_QUERY_CHARS} 个字符"
        )));
    }
    // 目录为空时后端会退回默认值，只有 Codex 没有可用默认目录，必须显式给出。
    if request.provider == "codex" && request.dirs.codex_dir.trim().is_empty() {
        return Err(AppError::Other("Codex 目录不能为空".to_string()));
    }
    if request.query.chars().any(char::is_control) {
        return Err(AppError::Other(
            "全文搜索关键词不能包含控制字符".to_string(),
        ));
    }
    Ok(())
}

fn run_job(job: SearchJob, request: SearchRequest) {
    let result = execute_search(&job, &request);
    let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
    match result {
        Ok(()) if job.cancel.load(Ordering::Acquire) => {
            status.state = "cancelled".to_string();
        }
        Ok(()) => {
            status.state = "completed".to_string();
        }
        Err(AppError::Cancelled) => {
            status.state = "cancelled".to_string();
        }
        Err(error) => {
            status.state = "failed".to_string();
            status.error = Some(error.to_string());
        }
    }
}

fn execute_search(job: &SearchJob, request: &SearchRequest) -> AppResult<()> {
    let sessions = if let Some(scopes) = &request.scopes {
        let mut found = Vec::new();
        for scope in scopes {
            if job.cancel.load(Ordering::Acquire) {
                return Err(AppError::Cancelled);
            }
            if scope.rollout_paths.is_empty() {
                continue;
            }
            let paths = scope
                .rollout_paths
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            if matches!(
                scope.provider.as_str(),
                "qoder" | "workbuddy" | "grok" | "pi" | "dsh" | "hermes" | "zcode"
            ) {
                let configured = match scope.provider.as_str() {
                    "qoder" => request.dirs.qoder_dir.as_deref(),
                    "workbuddy" => request.dirs.workbuddy_dir.as_deref(),
                    "grok" => request.dirs.grok_dir.as_deref(),
                    "pi" => request.dirs.pi_dir.as_deref(),
                    "dsh" => request.dirs.dsh_dir.as_deref(),
                    "hermes" => request.dirs.hermes_dir.as_deref(),
                    "zcode" => request.dirs.zcode_dir.as_deref(),
                    _ => None,
                };
                let root = std::path::Path::new(configured.unwrap_or(""));
                if !root.is_absolute() {
                    return Err(AppError::Path("搜索来源必须为绝对路径".into()));
                }
                // The selected paths are the scope. An unrelated unreadable file must
                // not prevent healthy selected transcripts from being searched.
                for path in &paths {
                    let result = match scope.provider.as_str() {
                        "qoder" => crate::qoder_sessions::parse_session(
                            root,
                            std::path::Path::new(path),
                            Some(&job.cancel),
                        ),
                        "workbuddy" => crate::workbuddy_sessions::parse_session(
                            root,
                            std::path::Path::new(path),
                            Some(&job.cancel),
                        ),
                        "grok" => crate::grok_sessions::parse_session(
                            root,
                            std::path::Path::new(path),
                            Some(&job.cancel),
                        ),
                        "pi" => crate::pi_sessions::parse_session(
                            root,
                            std::path::Path::new(path),
                            Some(&job.cancel),
                        ),
                        "dsh" => crate::dsh_sessions::parse_session(
                            root,
                            std::path::Path::new(path),
                            Some(&job.cancel),
                        ),
                        "hermes" => {
                            crate::hermes_sessions::parse_session(root, path, Some(&job.cancel))
                        }
                        "zcode" => {
                            crate::zcode_sessions::parse_session(root, path, Some(&job.cancel))
                        }
                        _ => unreachable!(),
                    };
                    match result {
                        Ok(Some(session)) => found.push(session),
                        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                        result => {
                            let error = match result {
                                Err(error) => error.to_string(),
                                _ => "未找到有效会话元数据".into(),
                            };
                            let mut status = job.status.lock().unwrap_or_else(|e| e.into_inner());
                            status.failed_files += 1;
                            if status.failures.len() < 100 {
                                status.failures.push(format!("{path}: {error}"));
                            }
                        }
                    }
                }
                continue;
            }
            let listed = match sessions::list_sessions_cancellable_with_dirs(
                Some(scope.provider.clone()),
                request.dirs.clone(),
                &job.cancel,
            ) {
                Ok(listed) => listed,
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(error) => {
                    let mut status = job.status.lock().unwrap_or_else(|e| e.into_inner());
                    status.failed_files += paths.len();
                    if status.failures.len() < 100 {
                        status
                            .failures
                            .push(format!("{} 来源读取失败: {error}", scope.provider));
                    }
                    continue;
                }
            };
            let selected = listed
                .into_iter()
                .filter(|session| session_matches_scope(session, &paths))
                .collect::<Vec<_>>();
            let resolved = selected
                .iter()
                .map(|session| session.rollout_path.as_str())
                .collect::<HashSet<_>>();
            for missing in paths.difference(&resolved) {
                let mut status = job.status.lock().unwrap_or_else(|e| e.into_inner());
                status.failed_files += 1;
                if status.failures.len() < 100 {
                    status.failures.push(format!(
                        "{missing}: 文件已消失或无法读取会话摘要，请刷新来源"
                    ));
                }
            }
            found.extend(selected);
        }
        let mut seen = HashSet::new();
        found.retain(|session| {
            seen.insert((session.provider.clone(), session.rollout_path.clone()))
        });
        found
    } else {
        let rollout_paths = request
            .rollout_paths
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let sessions = sessions::list_sessions_cancellable_with_dirs(
            Some(request.provider.clone()),
            request.dirs.clone(),
            &job.cancel,
        )?
        .into_iter()
        .filter(|session| session_matches_scope(session, &rollout_paths))
        .collect::<Vec<_>>();
        sessions
    };

    if job.cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    // Cursor 的 Composer 会话拿不到便宜的体积估计（`rollout_bytes` 为 0），
    // 每个会话至少记 1，进度条就退化成"扫到第几个会话"而不是原地不动。
    let total_bytes = sessions
        .iter()
        .map(|session| session.rollout_bytes.max(1))
        .sum();
    {
        let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
        status.total_files = sessions.len();
        status.total_bytes = total_bytes;
    }

    let mut completed_bytes = 0u64;
    for session in sessions {
        if job.cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let scan = (|| {
            if request.scopes.is_some() {
                let root = if session.provider == "codex" {
                    request.dirs.codex_dir.as_str()
                } else if session.provider == "workbuddy" {
                    request.dirs.workbuddy_dir.as_deref().unwrap_or("")
                } else if session.provider == "grok" {
                    request.dirs.grok_dir.as_deref().unwrap_or("")
                } else if session.provider == "pi" {
                    request.dirs.pi_dir.as_deref().unwrap_or("")
                } else if session.provider == "dsh" {
                    request.dirs.dsh_dir.as_deref().unwrap_or("")
                } else if session.provider == "opencode" {
                    request.dirs.opencode_dir.as_deref().unwrap_or("")
                } else if session.provider == "qoder" {
                    request.dirs.qoder_dir.as_deref().unwrap_or("")
                } else {
                    request.dirs.claude_dir.as_deref().unwrap_or("")
                };
                if session.provider == "opencode" {
                    crate::opencode_sessions::validate_scope(
                        std::path::Path::new(root),
                        &session.rollout_path,
                    )?;
                } else if !matches!(session.provider.as_str(), "hermes" | "zcode") {
                    crate::path_safety::validate_descendant(
                        std::path::Path::new(root),
                        std::path::Path::new(&session.rollout_path),
                        crate::path_safety::EntryKind::File,
                        false,
                        "正文搜索文件",
                    )?;
                }
            }
            scan_session_checked(
                job,
                &session,
                &request.query,
                completed_bytes,
                request.scopes.is_some(),
            )
        })();
        let outcome = match scan {
            Ok(outcome) => outcome,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) if request.scopes.is_some() => {
                let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
                status.failed_files += 1;
                if status.failures.len() < 100 {
                    status
                        .failures
                        .push(format!("{}: {error}", session.rollout_path));
                }
                status.scanned_files += 1;
                completed_bytes = completed_bytes.saturating_add(session.rollout_bytes.max(1));
                status.scanned_bytes = completed_bytes.min(status.total_bytes);
                continue;
            }
            Err(error) => return Err(error),
        };
        if outcome.cancelled {
            return Ok(());
        }
        completed_bytes = completed_bytes.saturating_add(
            if outcome.missing {
                session.rollout_bytes
            } else {
                outcome.bytes_read
            }
            .max(1),
        );

        let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
        status.scanned_files += 1;
        status.scanned_bytes = completed_bytes.min(status.total_bytes);
        if outcome.missing {
            status.skipped_files += 1;
            continue;
        }
        if !outcome.matches.is_empty() {
            status.results.push(ContentSearchResult {
                session,
                matches: outcome.matches,
            });
            if status.results.len() >= MAX_MATCHING_SESSIONS {
                status.truncated = status.scanned_files < status.total_files;
                return Ok(());
            }
        }
    }
    Ok(())
}

fn session_matches_scope(session: &SessionSummary, rollout_paths: &HashSet<&str>) -> bool {
    rollout_paths.contains(session.rollout_path.as_str())
}

#[cfg(test)]
fn scan_session(
    job: &SearchJob,
    session: &SessionSummary,
    query: &str,
    completed_bytes: u64,
) -> AppResult<FileScanOutcome> {
    scan_session_checked(job, session, query, completed_bytes, false)
}

fn scan_session_checked(
    job: &SearchJob,
    session: &SessionSummary,
    query: &str,
    completed_bytes: u64,
    strict: bool,
) -> AppResult<FileScanOutcome> {
    // Restore branch/chunk based providers to the same sequence used by previews.
    match session.provider.as_str() {
        "grok" => {
            let events = crate::grok_sessions::events(&session.rollout_path, Some(&job.cancel))?;
            return scan_event_sequence(job, session, query, completed_bytes, events);
        }
        "dsh" => {
            return scan_event_sequence(
                job,
                session,
                query,
                completed_bytes,
                crate::dsh_sessions::events(&session.rollout_path, Some(&job.cancel))?,
            );
        }
        "hermes" => {
            return scan_event_sequence(
                job,
                session,
                query,
                completed_bytes,
                crate::hermes_sessions::events(&session.rollout_path, Some(&job.cancel))?,
            );
        }
        "zcode" => {
            return scan_event_sequence(
                job,
                session,
                query,
                completed_bytes,
                crate::zcode_sessions::events(&session.rollout_path, Some(&job.cancel))?,
            );
        }
        "pi" => {
            let events = crate::pi_sessions::events(&session.rollout_path, Some(&job.cancel))?;
            return scan_event_sequence(job, session, query, completed_bytes, events);
        }
        "opencode" => {
            let events =
                crate::opencode_sessions::load_preview_events_from_locator(&session.rollout_path)?;
            return scan_event_sequence(job, session, query, completed_bytes, events);
        }
        "cursor" => {
            let events =
                crate::cursor_sessions::load_preview_events_from_locator(&session.rollout_path)?;
            return scan_event_sequence(job, session, query, completed_bytes, events);
        }
        _ => {}
    }
    let file = match File::open(&session.rollout_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileScanOutcome {
                matches: Vec::new(),
                bytes_read: 0,
                cancelled: false,
                missing: true,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_index = 0usize;
    let mut event_offset = 0usize;
    let mut bytes_read = 0u64;
    let mut reported_bytes = 0u64;
    let mut matches = Vec::new();
    let escaped_query = json_string_content(query);

    loop {
        if job.cancel.load(Ordering::Acquire) {
            return Ok(FileScanOutcome {
                matches,
                bytes_read,
                cancelled: true,
                missing: false,
            });
        }
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(count as u64);
        if bytes_read.saturating_sub(reported_bytes) >= PROGRESS_STEP_BYTES {
            reported_bytes = bytes_read;
            let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
            status.scanned_bytes = completed_bytes
                .saturating_add(bytes_read)
                .min(status.total_bytes);
        }

        let current_line_index = line_index;
        line_index += 1;
        if line.trim().is_empty() {
            continue;
        }
        let current_offset = event_offset;
        event_offset += 1;
        if strict {
            serde_json::from_str::<Value>(&line).map_err(|error| {
                AppError::Other(format!(
                    "第 {} 行 JSON 无效: {error}",
                    current_line_index + 1
                ))
            })?;
        }
        if matches.len() >= MAX_MATCHES_PER_SESSION
            || !line_might_contain_query(&line, query, &escaped_query)
        {
            continue;
        }

        let Ok(raw) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(event) = classify_event(&session.provider, current_line_index, raw) else {
            continue;
        };
        let mixed_assistant_text = rollout::preview_event_has_assistant_text_tool_use(&event);
        if !rollout::preview_event_is_conversation(&event) && !mixed_assistant_text {
            continue;
        }
        let text = rollout::preview_event_text(&event);
        if find_query(&text, query).is_none() {
            continue;
        }
        matches.push(ContentSearchMatch {
            event_index: event.index,
            event_offset: current_offset,
            timestamp: event.timestamp,
            role: if mixed_assistant_text {
                "assistant".to_string()
            } else {
                event.role
            },
            snippet: make_snippet(&text, query),
        });
    }

    Ok(FileScanOutcome {
        matches,
        bytes_read,
        cancelled: false,
        missing: false,
    })
}

/// 在已经展开好的事件序列上做匹配，供没有行式存储的 provider 复用。
fn scan_event_sequence(
    job: &SearchJob,
    session: &SessionSummary,
    query: &str,
    completed_bytes: u64,
    events: Vec<crate::models::PreviewEvent>,
) -> AppResult<FileScanOutcome> {
    let total_events = events.len().max(1);
    let mut matches = Vec::new();

    for (event_offset, event) in events.into_iter().enumerate() {
        if job.cancel.load(Ordering::Acquire) {
            return Ok(FileScanOutcome {
                matches,
                bytes_read: 0,
                cancelled: true,
                missing: false,
            });
        }
        if event_offset % 128 == 0 {
            let approximate =
                session.rollout_bytes.saturating_mul(event_offset as u64) / total_events as u64;
            let mut status = job.status.lock().unwrap_or_else(|error| error.into_inner());
            status.scanned_bytes = completed_bytes
                .saturating_add(approximate)
                .min(status.total_bytes);
        }
        if matches.len() >= MAX_MATCHES_PER_SESSION {
            continue;
        }
        let mixed_assistant_text = rollout::preview_event_has_assistant_text_tool_use(&event);
        if !rollout::preview_event_is_conversation(&event) && !mixed_assistant_text {
            continue;
        }
        let text = rollout::preview_event_text(&event);
        if find_query(&text, query).is_none() {
            continue;
        }
        matches.push(ContentSearchMatch {
            event_index: event.index,
            event_offset,
            timestamp: event.timestamp,
            role: if mixed_assistant_text {
                "assistant".to_string()
            } else {
                event.role
            },
            snippet: make_snippet(&text, query),
        });
    }

    Ok(FileScanOutcome {
        matches,
        bytes_read: session.rollout_bytes,
        cancelled: false,
        missing: false,
    })
}

fn classify_event(provider: &str, index: usize, raw: Value) -> Option<crate::models::PreviewEvent> {
    match provider {
        "codex" => Some(rollout::classify_preview(index, raw)),
        // Cursor 与 OpenCode 都会先合成 Claude 形状的记录再分类。
        "qoder" => crate::qoder_sessions::classify_preview(index, raw),
        "workbuddy" => crate::workbuddy_sessions::classify_preview(index, raw),
        "claude" | "cursor" => crate::claude_sessions::classify_preview(index, raw),
        _ => None,
    }
}

fn find_query(text: &str, query: &str) -> Option<usize> {
    if query.is_ascii() {
        text.as_bytes()
            .windows(query.len())
            .position(|window| window.eq_ignore_ascii_case(query.as_bytes()))
    } else {
        text.find(query)
    }
}

fn json_string_content(text: &str) -> String {
    let encoded = serde_json::to_string(text).expect("serializing a string cannot fail");
    encoded[1..encoded.len() - 1].to_string()
}

fn line_might_contain_query(line: &str, query: &str, escaped_query: &str) -> bool {
    find_query(line, query).is_some()
        || (escaped_query != query && find_query(line, escaped_query).is_some())
        // A valid JSON string may encode any character as `\uXXXX`; parse such
        // lines before deciding so the raw-byte prefilter cannot hide content.
        || line.contains("\\u")
}

fn make_snippet(text: &str, query: &str) -> String {
    let Some(position) = find_query(text, query) else {
        return String::new();
    };
    let before = text[..position]
        .chars()
        .rev()
        .take(SNIPPET_BEFORE_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let match_and_after = text[position..]
        .chars()
        .take(query.chars().count() + SNIPPET_AFTER_CHARS)
        .collect::<String>();
    let mut snippet = compact_whitespace(&format!("{before}{match_and_after}"));
    if before.chars().count() == SNIPPET_BEFORE_CHARS {
        snippet.insert_str(0, "...");
    }
    if text[position..].chars().count() > query.chars().count() + SNIPPET_AFTER_CHARS {
        snippet.push_str("...");
    }
    snippet
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn empty_workbench_scope_never_discovers_default_sources() {
        let job = test_job();
        execute_search(
            &job,
            &SearchRequest {
                provider: "workbench".into(),
                dirs: ProviderDirs::default(),
                query: "needle".into(),
                rollout_paths: Vec::new(),
                scopes: Some(Vec::new()),
            },
        )
        .unwrap();
        let status = job.status.lock().unwrap();
        assert_eq!(status.total_files, 0);
        assert!(status.results.is_empty());
    }

    #[test]
    fn workbench_reports_invalid_json_even_without_query_match() {
        let path = temp_file("invalid-workbench", &[json!({"type":"session_meta"})]);
        fs::write(&path, "{broken json\n").unwrap();
        let result = scan_session_checked(&test_job(), &session("codex", &path), "needle", 0, true);
        assert!(result.err().unwrap().to_string().contains("第 1 行"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn workbench_scope_excludes_other_files_and_reports_missing_members() {
        let seed = temp_file("workbench-scope", &[]);
        let root = seed.parent().unwrap();
        let project = root.join("projects");
        fs::create_dir_all(&project).unwrap();
        let included = project.join("included.jsonl");
        let excluded = project.join("excluded.jsonl");
        let missing = project.join("missing.jsonl");
        for (path, id) in [(&included, "included"), (&excluded, "excluded")] {
            fs::write(path, format!("{{\"sessionId\":\"{id}\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"needle\"}}}}\n")).unwrap();
        }
        let job = test_job();
        execute_search(
            &job,
            &SearchRequest {
                provider: "workbench".into(),
                dirs: ProviderDirs {
                    claude_dir: Some(root.to_string_lossy().into_owned()),
                    ..Default::default()
                },
                query: "needle".into(),
                rollout_paths: Vec::new(),
                scopes: Some(vec![ContentSearchScope {
                    provider: "claude".into(),
                    rollout_paths: vec![
                        included.to_string_lossy().into_owned(),
                        missing.to_string_lossy().into_owned(),
                    ],
                }]),
            },
        )
        .unwrap();
        let status = job.status.lock().unwrap();
        assert_eq!(status.results.len(), 1);
        assert_eq!(status.results[0].session.id, "included");
        assert_eq!(status.failed_files, 1);
        assert!(status.failures[0].contains("missing.jsonl"));
        drop(status);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_provider_searches_match_preview_offsets_and_exclude_inactive_content() {
        let seed = temp_file("new-provider-scope", &[]);
        let root = seed.parent().unwrap();
        let wb = root.join("projects/demo/workbuddy.jsonl");
        let grok = root.join("sessions/project/s/updates.jsonl");
        let pi = root.join("sessions/project/pi.jsonl");
        fs::create_dir_all(wb.parent().unwrap()).unwrap();
        fs::create_dir_all(grok.parent().unwrap()).unwrap();
        fs::write(
            &wb,
            include_str!("../tests/fixtures/workbuddy/sample.jsonl"),
        )
        .unwrap();
        fs::write(
            grok.with_file_name("summary.json"),
            include_str!("../tests/fixtures/grok/summary.json"),
        )
        .unwrap();
        let chunk = |kind: &str, text: &str, index: usize| json!({"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":kind,"_meta":{"promptIndex":index},"content":{"type":"text","text":text}}}});
        let body = [
            chunk("user_message_chunk", "obsolete needle", 0),
            json!({"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"rewind_marker","target_prompt_index":0}}}),
            chunk("user_message_chunk", "nee", 0),
            chunk("user_message_chunk", "dle", 0),
            chunk("agent_message_chunk", "answer needle", 0),
        ];
        fs::write(
            &grok,
            body.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let body = [
            json!({"type":"session","version":3,"id":"pi"}),
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"needle"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":"obsolete needle"}}),
            json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":"answer needle"}}),
        ];
        fs::write(
            &pi,
            body.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let dirs = ProviderDirs {
            workbuddy_dir: Some(root.to_string_lossy().into_owned()),
            grok_dir: Some(root.to_string_lossy().into_owned()),
            pi_dir: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let job = test_job();
        execute_search(
            &job,
            &SearchRequest {
                provider: "workbench".into(),
                dirs,
                query: "needle".into(),
                rollout_paths: vec![],
                scopes: Some(vec![
                    ContentSearchScope {
                        provider: "workbuddy".into(),
                        rollout_paths: vec![wb.to_string_lossy().into_owned()],
                    },
                    ContentSearchScope {
                        provider: "grok".into(),
                        rollout_paths: vec![grok.to_string_lossy().into_owned()],
                    },
                    ContentSearchScope {
                        provider: "pi".into(),
                        rollout_paths: vec![pi.to_string_lossy().into_owned()],
                    },
                ]),
            },
        )
        .unwrap();
        let status = job.status.lock().unwrap();
        assert_eq!(status.failed_files, 0, "{:?}", status.failures);
        assert_eq!(status.results.len(), 3);
        for result in &status.results {
            assert_eq!(result.matches.len(), 2);
            for hit in &result.matches {
                assert!(!hit.snippet.contains("obsolete"));
                let events = crate::rollout::preview_session_range(
                    Some(result.session.provider.clone()),
                    result.session.rollout_path.clone(),
                    hit.event_offset,
                    1,
                )
                .unwrap();
                assert_eq!(events[0].index, hit.event_index);
                assert!(crate::rollout::preview_event_text(&events[0]).contains("needle"));
            }
        }
        drop(status);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qoder_workbench_search_offsets_match_preview() {
        let seed = temp_file("qoder-scope", &[]);
        let root = seed.parent().unwrap();
        let project = root.join("projects/demo");
        fs::create_dir_all(&project).unwrap();
        let path = project.join("session.jsonl");
        let body = [
            json!({"sessionId":"qoder","isSidechain":true,"type":"user","message":{"role":"user","content":"needle hidden"}}),
            json!({"sessionId":"qoder","type":"assistant","message":{"role":"assistant","content":[{"type":"output_text","text":"needle visible"},{"type":"unknown","text":"needle unknown"}]}})
        ].iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n");
        fs::write(&path, body).unwrap();
        let job = test_job();
        execute_search(
            &job,
            &SearchRequest {
                provider: "workbench".into(),
                dirs: ProviderDirs {
                    qoder_dir: Some(root.to_string_lossy().into_owned()),
                    ..Default::default()
                },
                query: "needle".into(),
                rollout_paths: vec![],
                scopes: Some(vec![ContentSearchScope {
                    provider: "qoder".into(),
                    rollout_paths: vec![path.to_string_lossy().into_owned()],
                }]),
            },
        )
        .unwrap();
        let status = job.status.lock().unwrap();
        assert_eq!(status.results.len(), 1);
        assert_eq!(status.results[0].matches.len(), 1);
        let hit = &status.results[0].matches[0];
        assert_eq!(hit.event_index, 1);
        assert_eq!(hit.event_offset, 1);
        assert!(!hit.snippet.contains("unknown"));
        let events =
            crate::qoder_sessions::preview_range(path.to_str().unwrap(), hit.event_offset, 1)
                .unwrap();
        assert_eq!(events[0].index, hit.event_index);
        drop(status);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qoder_scoped_search_isolates_unreadable_files() {
        let seed = temp_file("qoder-file-failures", &[]);
        let root = seed.parent().unwrap();
        let project = root.join("projects/demo");
        fs::create_dir_all(&project).unwrap();
        let healthy = project.join("healthy.jsonl");
        let broken = project.join("broken.jsonl");
        fs::write(&healthy, json!({"sessionId":"healthy","type":"user","message":{"role":"user","content":"needle"}}).to_string()).unwrap();
        fs::write(&broken, [0xff, 0xfe, b'\n']).unwrap();
        for include_broken in [false, true] {
            let mut rollout_paths = vec![healthy.to_string_lossy().into_owned()];
            if include_broken {
                rollout_paths.push(broken.to_string_lossy().into_owned());
            }
            let job = test_job();
            execute_search(
                &job,
                &SearchRequest {
                    provider: "workbench".into(),
                    dirs: ProviderDirs {
                        qoder_dir: Some(root.to_string_lossy().into_owned()),
                        ..Default::default()
                    },
                    query: "needle".into(),
                    rollout_paths: vec![],
                    scopes: Some(vec![ContentSearchScope {
                        provider: "qoder".into(),
                        rollout_paths,
                    }]),
                },
            )
            .unwrap();
            let status = job.status.lock().unwrap();
            assert_eq!(status.results.len(), 1);
            assert_eq!(status.results[0].session.id, "healthy");
            assert_eq!(status.failed_files, usize::from(include_broken));
            assert_eq!(status.failures.len(), usize::from(include_broken));
            if include_broken {
                assert!(status.failures[0].contains("broken.jsonl"));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_file(name: &str, lines: &[Value]) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cc-sessions-search-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("session.jsonl");
        let body = lines
            .iter()
            .map(|line| serde_json::to_string(line).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}\n")).expect("fixture");
        path
    }

    fn session(provider: &str, path: &std::path::Path) -> SessionSummary {
        SessionSummary {
            provider: provider.to_string(),
            id: "search-session".to_string(),
            resume_command: String::new(),
            rollout_path: path.to_string_lossy().into_owned(),
            cwd: "/tmp/project".to_string(),
            cwd_display: "project".to_string(),
            title: "Search session".to_string(),
            first_user_message: "first".to_string(),
            model: None,
            reasoning_effort: None,
            source: None,
            agent_nickname: None,
            agent_role: None,
            conversion_origin: None,
            tokens_used: 0,
            created_at: 0,
            updated_at: 0,
            archived: false,
            git_branch: None,
            rollout_bytes: fs::metadata(path).expect("metadata").len(),
            logs_count: 0,
            has_backup: false,
        }
    }

    fn test_job() -> SearchJob {
        SearchJob {
            id: 1,
            cancel: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(ContentSearchStatus {
                failures: Vec::new(),
                failed_files: 0,
                job_id: 1,
                state: "running".to_string(),
                query: "needle".to_string(),
                scanned_files: 0,
                total_files: 1,
                skipped_files: 0,
                scanned_bytes: 0,
                total_bytes: u64::MAX,
                results: Vec::new(),
                truncated: false,
                error: None,
            })),
        }
    }

    #[test]
    fn searches_later_codex_conversation_messages_only() {
        let path = temp_file(
            "codex",
            &[
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"first prompt"}]}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"later needle prompt"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"later needle prompt"}]}}),
                json!({"type":"response_item","payload":{"type":"function_call_output","output":"needle in tool output"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"later needle answer"}]}}),
            ],
        );
        let result =
            scan_session(&test_job(), &session("codex", &path), "needle", 0).expect("search");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matches[0].role, "user");
        assert_eq!(result.matches[0].event_index, 2);
        assert_eq!(result.matches[1].role, "assistant");
        assert_eq!(result.matches[1].event_index, 4);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    fn searches_later_claude_user_messages() {
        let path = temp_file(
            "claude",
            &[
                json!({"type":"user","message":{"role":"user","content":"first prompt"}}),
                json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"needle reasoning"}]}}),
                json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"needle_tool","input":{"query":"needle"}}]}}),
                json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"assistant needle answer"},{"type":"tool_use","name":"Search","input":{}}]}}),
                json!({"type":"user","message":{"role":"user","content":"later needle prompt"}}),
            ],
        );
        let result =
            scan_session(&test_job(), &session("claude", &path), "needle", 0).expect("search");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matches[0].role, "assistant");
        assert_eq!(result.matches[0].event_index, 3);
        assert_eq!(result.matches[1].role, "user");
        assert_eq!(result.matches[1].event_index, 4);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    fn searches_opencode_sqlite_conversation_text_only() -> AppResult<()> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cc-sessions-search-opencode-{unique}"));
        fs::create_dir_all(&root)?;
        let connection =
            rusqlite::Connection::open(crate::opencode_sessions::database_path(&root))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE session (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT, slug TEXT NOT NULL, directory TEXT NOT NULL, title TEXT NOT NULL, version TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE, session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);",
        )?;
        connection.execute(
            "INSERT INTO session VALUES ('ses_search', 'global', NULL, 'slug', 'F:\\project', 'Search', '1.0', 1000, 4000, NULL)",
            [],
        )?;
        connection.execute(
            "INSERT INTO message VALUES ('msg_user', 'ses_search', 1000, 1000, ?1)",
            [json!({"role":"user"}).to_string()],
        )?;
        connection.execute(
            "INSERT INTO message VALUES ('msg_process', 'ses_search', 2000, 2000, ?1)",
            [json!({"role":"assistant","parentID":"msg_user","finish":"tool-calls"}).to_string()],
        )?;
        connection.execute(
            "INSERT INTO message VALUES ('msg_final', 'ses_search', 3000, 3000, ?1)",
            [json!({"role":"assistant","parentID":"msg_user","finish":"stop"}).to_string()],
        )?;
        for (id, message, created, data) in [
            (
                "part_user",
                "msg_user",
                1000,
                json!({"type":"text","text":"visible needle prompt"}),
            ),
            (
                "part_reasoning",
                "msg_process",
                2000,
                json!({"type":"reasoning","text":"hidden needle reasoning"}),
            ),
            (
                "part_tool",
                "msg_process",
                2100,
                json!({"type":"tool","callID":"call_1","tool":"needle_tool","state":{"input":{"query":"needle"}}}),
            ),
            (
                "part_final",
                "msg_final",
                3000,
                json!({"type":"text","text":"visible needle answer"}),
            ),
        ] {
            connection.execute(
                "INSERT INTO part VALUES (?1, ?2, 'ses_search', ?3, ?3, ?4)",
                rusqlite::params![id, message, created, data.to_string()],
            )?;
        }
        drop(connection);

        let session = crate::opencode_sessions::list_sessions(&root)?
            .into_iter()
            .next()
            .expect("OpenCode fixture session");
        let result = scan_session(&test_job(), &session, "needle", 0)?;

        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matches[0].role, "user");
        assert_eq!(result.matches[0].snippet, "visible needle prompt");
        assert_eq!(result.matches[1].role, "assistant");
        assert_eq!(result.matches[1].snippet, "visible needle answer");
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn searches_text_that_is_escaped_in_json() {
        let path = temp_file(
            "escaped",
            &[
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"say \"needle\" here"}]}}),
            ],
        );
        let result =
            scan_session(&test_job(), &session("codex", &path), "\"needle\"", 0).expect("search");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].snippet, "say \"needle\" here");
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    fn searches_unicode_escaped_json_content() {
        let path = temp_file("unicode-escaped", &[]);
        fs::write(
            &path,
            concat!(
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"\u4f60\u597d"}]}}"#,
                "\n"
            ),
        )
        .expect("fixture");

        let result =
            scan_session(&test_job(), &session("codex", &path), "你好", 0).expect("search");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].snippet, "你好");
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    fn requested_rollout_paths_are_the_authoritative_backend_scope() {
        let included_path = temp_file("scope-included", &[json!({"type":"session_meta"})]);
        let excluded_path = temp_file("scope-excluded", &[json!({"type":"session_meta"})]);
        let mut included = session("codex", &included_path);
        included.archived = true;
        included.agent_role = Some("worker".to_string());
        let excluded = session("codex", &excluded_path);
        let rollout_paths = HashSet::from([included.rollout_path.as_str()]);

        assert!(session_matches_scope(&included, &rollout_paths));
        assert!(!session_matches_scope(&excluded, &rollout_paths));
        fs::remove_dir_all(included_path.parent().expect("parent")).ok();
        fs::remove_dir_all(excluded_path.parent().expect("parent")).ok();
    }

    #[test]
    fn marks_missing_rollout_without_failing_the_search() {
        let path = temp_file("missing", &[json!({"type":"session_meta"})]);
        let missing_session = session("codex", &path);
        fs::remove_dir_all(path.parent().expect("parent")).expect("remove fixture");

        let outcome = scan_session(&test_job(), &missing_session, "needle", 0)
            .expect("missing rollout should be a counted search outcome");

        assert!(outcome.missing);
        assert!(!outcome.cancelled);
        assert_eq!(outcome.bytes_read, 0);
        assert!(outcome.matches.is_empty());
    }

    #[test]
    fn cancellation_stops_before_claude_discovery_reads_session_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cc-sessions-cancel-{unique}"));
        let claude_dir = root.join("claude");
        let project_dir = claude_dir.join("projects").join("project");
        fs::create_dir_all(&project_dir).expect("project dir");
        fs::write(project_dir.join("invalid.jsonl"), [0xff]).expect("invalid fixture");

        let job = test_job();
        job.cancel.store(true, Ordering::Release);
        let request = SearchRequest {
            provider: "claude".to_string(),
            dirs: ProviderDirs {
                backup_dir: None,
                codex_dir: root.join("codex").to_string_lossy().into_owned(),
                claude_dir: Some(claude_dir.to_string_lossy().into_owned()),
                opencode_dir: Some(root.join("opencode").to_string_lossy().into_owned()),
                cursor_dir: Some(root.join("cursor").to_string_lossy().into_owned()),
                cursor_agent_dir: Some(root.join("cursor-agent").to_string_lossy().into_owned()),
                qoder_dir: None,
                workbuddy_dir: None,
                grok_dir: None,
                dsh_dir: None,
                hermes_dir: None,
                zcode_dir: None,
                pi_dir: None,
            },
            query: "needle".to_string(),
            rollout_paths: Vec::new(),
            scopes: None,
        };

        let result = execute_search(&job, &request);
        fs::remove_dir_all(root).ok();

        assert!(matches!(result, Err(AppError::Cancelled)));
    }

    #[test]
    fn active_search_is_discoverable_only_while_running() {
        let manager = SearchManager::new();
        let job = test_job();
        *manager
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(job.clone());

        assert_eq!(manager.active().expect("running job").job_id, job.id);
        job.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .state = "completed".to_string();
        assert!(manager.active().is_none());
    }
}
