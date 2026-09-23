//! Shared read-only source identity and projections; never creates or migrates native data.
use crate::{
    error::{AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static ROOTS: OnceLock<Mutex<HashSet<(String, PathBuf)>>> = OnceLock::new();
pub fn register(provider: &str, root: &Path) {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((provider.into(), root.to_path_buf()));
}
pub fn validate_file(root: &Path, path: &Path) -> AppResult<()> {
    if !root.is_absolute()
        || root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::Path("来源必须为无路径穿越的绝对目录".into()));
    }
    for ancestor in root.ancestors() {
        if crate::path_safety::metadata_is_link_or_reparse(&std::fs::symlink_metadata(ancestor)?) {
            return Err(AppError::Path("来源不能经过链接或 junction".into()));
        }
    }
    crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "只读来源",
    )?;
    Ok(())
}
pub fn open_db(root: &Path, path: &Path) -> AppResult<Connection> {
    validate_file(root, path)?;
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        crate::path_safety::validate_descendant(
            root,
            Path::new(&sidecar),
            crate::path_safety::EntryKind::File,
            true,
            "只读数据库 sidecar",
        )?;
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(std::time::Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; BEGIN DEFERRED;")?;
    Ok(db)
}
pub fn locator(provider: &str, root: &Path, id: &str) -> String {
    format!(
        "{provider}:{}",
        URL_SAFE_NO_PAD.encode(json!({"root":root.to_string_lossy(),"id":id}).to_string())
    )
}
pub fn decode(provider: &str, value: &str) -> AppResult<(PathBuf, String)> {
    let data = value
        .strip_prefix(&format!("{provider}:"))
        .ok_or_else(|| AppError::Path("来源定位符无效".into()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|_| AppError::Path("来源定位符无法解码".into()))?;
    let v: Value = serde_json::from_slice(&bytes)?;
    let root = PathBuf::from(v["root"].as_str().unwrap_or(""));
    let id = v["id"].as_str().unwrap_or("").to_owned();
    if !root.is_absolute() || id.is_empty() {
        return Err(AppError::Path("来源定位符无效".into()));
    }
    Ok((root, id))
}
pub fn registered(provider: &str, value: &str) -> AppResult<(PathBuf, String)> {
    let (root, id) = decode(provider, value)?;
    if !ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&(provider.into(), root.clone()))
    {
        return Err(AppError::Path("请先扫描当前来源，再预览会话".into()));
    }
    Ok((root, id))
}
pub fn timestamp(millis: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(millis).map(|d| d.to_rfc3339())
}
pub fn event(index: usize, role: &str, content: Value, millis: i64, native: Value) -> PreviewEvent {
    let raw = json!({"type":role,"timestamp":timestamp(millis),"message":{"role":role,"content":content},"native":native});
    crate::claude_sessions::classify_preview(index, raw.clone()).unwrap_or(PreviewEvent {
        index,
        role: "meta".into(),
        kind: "meta".into(),
        text_summary: String::new(),
        timestamp: timestamp(millis).unwrap_or_default(),
        raw,
    })
}
pub fn text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.into();
    }
    content
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|b| {
                    matches!(
                        b["type"].as_str(),
                        Some("text" | "input_text" | "output_text")
                    )
                })
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
pub fn summary(
    provider: &str,
    id: String,
    path: String,
    cwd: String,
    title: String,
    created: i64,
    updated: i64,
    events: &[PreviewEvent],
    bytes: u64,
) -> SessionSummary {
    let first = events
        .iter()
        .find(|e| e.role == "user")
        .map(crate::rollout::preview_event_text)
        .unwrap_or_default();
    SessionSummary {
        provider: provider.into(),
        id,
        rollout_path: path,
        cwd_display: crate::paths::basename_display(&cwd),
        cwd,
        title: if title.is_empty() {
            first.chars().take(100).collect()
        } else {
            title
        },
        first_user_message: first,
        model: None,
        reasoning_effort: None,
        source: Some("readonly".into()),
        agent_nickname: None,
        agent_role: None,
        conversion_origin: None,
        tokens_used: 0,
        created_at: created,
        updated_at: updated,
        archived: false,
        git_branch: None,
        rollout_bytes: bytes,
        logs_count: 0,
        has_backup: false,
        resume_command: String::new(),
    }
}
pub fn meta(s: SessionSummary) -> SessionMetaBrief {
    SessionMetaBrief {
        id: Some(s.id),
        timestamp: timestamp(s.created_at * 1000),
        cwd: Some(s.cwd),
        originator: Some(s.provider),
        cli_version: None,
        source: s.source,
        model_provider: None,
    }
}
pub fn columns(db: &Connection, table: &str) -> AppResult<HashSet<String>> {
    // Only internal constant table names reach this function.
    Ok(db
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?)
}
pub fn optional(cols: &HashSet<String>, name: &str, fallback: &str) -> String {
    if cols.contains(name) {
        name.into()
    } else {
        fallback.into()
    }
}
