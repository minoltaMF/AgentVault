//! Read-only Cline CLI/Desktop v1 artifacts. Projection adapted from agent-sessions
//! ab439f13211e56b809f4a917d5c38e80d2bb5258 (MIT; fixture directory LICENSE).
use crate::{
    error::{ensure_not_cancelled, AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
    readonly_source as ro,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};
static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
const LIMIT: u64 = 50 * 1024 * 1024;
pub fn is_main_transcript(root: &Path, path: &Path) -> bool {
    [
        root.to_path_buf(),
        root.join("sessions"),
        root.join("data/sessions"),
    ]
    .iter()
    .any(|base| {
        path.strip_prefix(base).ok().is_some_and(|p| {
            p.components().count() == 2
                && p.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && p.extension().and_then(|s| s.to_str()) == Some("json")
                && p.file_stem() == p.parent().and_then(Path::file_name)
                && !p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .ends_with(".messages")
        })
    })
}
fn read_json(root: &Path, path: &Path, cancel: Option<&AtomicBool>) -> AppResult<Value> {
    ensure_not_cancelled(cancel)?;
    ro::validate_file(root, path)?;
    let mut reader = fs::File::open(path)?.take(LIMIT + 1);
    let mut bytes = Vec::new();
    let mut buf = [0; 65536];
    loop {
        ensure_not_cancelled(cancel)?;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    if bytes.len() as u64 > LIMIT {
        return Err(AppError::Other("Cline 文件超过 50 MiB 读取限制".into()));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn millis(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}
fn read(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<(Value, Vec<PreviewEvent>, u64, i64)> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path(
            "仅支持 Cline CLI/Desktop sessions 中的 manifest".into(),
        ));
    }
    let manifest = read_json(root, path, cancel)?;
    let id = manifest["session_id"]
        .as_str()
        .ok_or_else(|| AppError::Other("Cline manifest 缺少 session_id".into()))?;
    if manifest["version"] != 1 || Some(id) != path.file_stem().and_then(|s| s.to_str()) {
        return Err(AppError::Other("Cline manifest 版本或身份不匹配".into()));
    }
    // Never follow messages_path: it may refer to a different machine or profile.
    let companion = path.with_file_name(format!("{id}.messages.json"));
    let transcript = read_json(root, &companion, cancel)?;
    let bytes = fs::metadata(path)?
        .len()
        .saturating_add(fs::metadata(&companion)?.len());
    if bytes > LIMIT {
        return Err(AppError::Other("Cline 会话文件合计超过 50 MiB".into()));
    }
    if transcript["version"] != 1
        || transcript
            .get("sessionId")
            .or_else(|| transcript.get("session_id"))
            .and_then(Value::as_str)
            != Some(id)
    {
        return Err(AppError::Other("Cline companion 版本或身份不匹配".into()));
    }
    let rows = transcript["messages"]
        .as_array()
        .ok_or_else(|| AppError::Other("Cline messages 必须为数组".into()))?;
    let mut events = Vec::new();
    let mut updated = millis(&transcript["updated_at"]).max(millis(&manifest["ended_at"]));
    for (message_index, row) in rows.iter().enumerate() {
        ensure_not_cancelled(cancel)?;
        let role = row["role"].as_str().unwrap_or("meta");
        let time = row["ts"].as_i64().unwrap_or(0);
        updated = updated.max(time);
        let blocks = row["content"].as_array().ok_or_else(|| {
            AppError::Other(format!("Cline message {message_index} content 必须为数组"))
        })?;
        for (block_index, block) in blocks.iter().enumerate() {
            ensure_not_cancelled(cancel)?;
            let kind = block["type"].as_str().unwrap_or("");
            let role = match kind {
                "tool_result" => "user",
                "tool_use" | "thinking" | "redacted_thinking" => "assistant",
                "text" if matches!(role, "user" | "assistant") => role,
                _ => "meta",
            };
            events.push(ro::event(
                events.len(),
                role,
                json!([block]),
                time,
                json!({"message_index":message_index,"block_index":block_index,"id":row["id"]}),
            ));
        }
    }
    Ok((manifest, events, bytes, updated))
}
pub fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    if !is_main_transcript(root, path) {
        return Ok(None);
    }
    let (m, events, bytes, updated) = read(root, path, cancel)?;
    let started = millis(&m["started_at"]);
    let mut s = ro::summary(
        "cline",
        m["session_id"].as_str().unwrap().into(),
        path.to_string_lossy().into_owned(),
        m["cwd"]
            .as_str()
            .or_else(|| m["workspace_root"].as_str())
            .unwrap_or("")
            .into(),
        m.pointer("/metadata/title")
            .and_then(Value::as_str)
            .or_else(|| m["prompt"].as_str())
            .unwrap_or("")
            .into(),
        started / 1000,
        updated.max(started) / 1000,
        &events,
        bytes,
    );
    s.model = m["model"].as_str().map(str::to_owned);
    s.source = Some(m["source"].as_str().unwrap_or("cli/desktop").into());
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.into());
    Ok(Some(s))
}
fn root_for(path: &Path) -> AppResult<PathBuf> {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|r| is_main_transcript(r, path))
        .cloned()
        .ok_or_else(|| AppError::Path("请先扫描 Cline 来源".into()))
}
pub fn events(path: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let p = Path::new(path);
    Ok(read(&root_for(p)?, p, cancel)?.1)
}
pub fn preview_range(path: &str, offset: usize, limit: usize) -> AppResult<Vec<PreviewEvent>> {
    Ok(events(path, None)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}
pub fn preview_meta(path: &str) -> AppResult<SessionMetaBrief> {
    let p = Path::new(path);
    Ok(ro::meta(
        parse_session(&root_for(p)?, p, None)?
            .ok_or_else(|| AppError::Path("Cline 会话已变化".into()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(desktop: bool) -> (crate::readonly_source::test_support::TempDir, PathBuf) {
        let t = crate::readonly_source::test_support::tempdir().unwrap();
        let (id, manifest, messages) = if desktop {
            ("cline-desktop-continued",include_str!("../tests/fixtures/cline/desktop_continued/cline-desktop-continued.json"),include_str!("../tests/fixtures/cline/desktop_continued/cline-desktop-continued.messages.json"))
        } else {
            (
                "cline-cli-tool",
                include_str!("../tests/fixtures/cline/cli_tool/cline-cli-tool.json"),
                include_str!("../tests/fixtures/cline/cli_tool/cline-cli-tool.messages.json"),
            )
        };
        let dir = t.path().join("sessions").join(id);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.json"));
        fs::write(&path, manifest).unwrap();
        fs::write(dir.join(format!("{id}.messages.json")), messages).unwrap();
        (t, path)
    }
    #[test]
    fn cline_upstream_fixtures_keep_dialogue_and_tool_boundaries() {
        for desktop in [false, true] {
            let (t, path) = fixture(desktop);
            let original = fs::read(&path).unwrap();
            let summary = parse_session(t.path(), &path, None).unwrap().unwrap();
            let events = events(path.to_str().unwrap(), None).unwrap();
            assert_eq!(summary.provider, "cline");
            assert!(!summary.first_user_message.is_empty());
            if !desktop {
                assert!(events.iter().any(|e| e.role == "tool_result"));
                assert!(events.iter().any(|e| e.role == "reasoning"));
            }
            assert!(events.iter().any(|e| e.role == "assistant"));
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(events[1].index, 1);
        }
    }
    #[test]
    fn cline_refuses_mismatched_missing_and_cancelled_companion() {
        let (t, path) = fixture(false);
        let companion = path.with_file_name("cline-cli-tool.messages.json");
        let mut data: Value = serde_json::from_slice(&fs::read(&companion).unwrap()).unwrap();
        data["sessionId"] = "foreign".into();
        fs::write(&companion, data.to_string()).unwrap();
        assert!(parse_session(t.path(), &path, None).is_err());
        fs::remove_file(&companion).unwrap();
        assert!(parse_session(t.path(), &path, None).is_err());
        assert!(matches!(
            parse_session(t.path(), &path, Some(&AtomicBool::new(true))),
            Err(AppError::Cancelled)
        ));
    }
    #[test]
    fn cline_preview_requires_registered_root_and_does_not_follow_messages_path() {
        let (t, path) = fixture(false);
        assert!(events(path.to_str().unwrap(), None).is_err());
        let mut m: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        m["messages_path"] = "/private/foreign.json".into();
        fs::write(&path, m.to_string()).unwrap();
        assert!(parse_session(t.path(), &path, None).unwrap().is_some());
        assert!(!is_main_transcript(
            t.path(),
            &path.with_file_name("cline-cli-tool.messages.json")
        ));
    }
}

#[cfg(all(test, unix))]
mod link_tests {
    use super::*;
    #[test]
    fn refuses_symlinked_companion_without_reading_it() {
        let t = crate::readonly_source::test_support::tempdir().unwrap();
        let dir = t.path().join("sessions/a");
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("a.json");
        fs::write(&manifest, r#"{"version":1,"session_id":"a"}"#).unwrap();
        let target = t.path().join("foreign.json");
        fs::write(&target, r#"{"version":1,"sessionId":"a","messages":[]}"#).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("a.messages.json")).unwrap();
        assert!(parse_session(t.path(), &manifest, None).is_err());
    }
}
