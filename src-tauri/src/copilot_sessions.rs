//! GitHub Copilot CLI event-log projection; excludes VS Code Copilot Chat.
//! Provenance and supported persistence format: tests/fixtures/copilot/README.md.
use crate::{
    error::{ensure_not_cancelled, AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
    readonly_source as ro,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};
static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
pub fn is_main_transcript(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root.join("session-state")) else {
        return false;
    };
    let parts: Vec<_> = relative.components().collect();
    parts.iter().all(|c| matches!(c, Component::Normal(_)))
        && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
        && (parts.len() == 1
            || (parts.len() == 2 && path.file_name().is_some_and(|s| s == "events.jsonl")))
}
fn invalid(message: &str) -> AppError {
    AppError::Other(format!("GitHub Copilot CLI: {message}"))
}
fn root_for(path: &Path) -> AppResult<PathBuf> {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|r| is_main_transcript(r, path))
        .cloned()
        .ok_or_else(|| AppError::Path("请先扫描 GitHub Copilot CLI 来源".into()))
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
) -> AppResult<(Vec<Value>, Vec<PreviewEvent>)> {
    ensure_not_cancelled(cancel)?;
    if !is_main_transcript(root, path) {
        return Err(AppError::Path("仅支持 Copilot session-state 会话".into()));
    }
    ro::validate_file(root, path)?;
    let mut reader = BufReader::new(fs::File::open(path)?.take(128 * 1024 * 1024 + 1));
    let mut line = String::new();
    let mut rows = Vec::<Value>::new();
    let mut seen = HashSet::new();
    let mut total = 0;
    loop {
        ensure_not_cancelled(cancel)?;
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            break;
        }
        total += count;
        if total > 128 * 1024 * 1024 {
            return Err(invalid("会话超过 128 MiB 安全读取上限"));
        }
        if !line.ends_with('\n') {
            return Err(invalid("尾行尚未写完，请稍后刷新"));
        }
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(&line)?;
        let id = row["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("事件 ID 无效"))?;
        if row["type"].as_str().is_none_or(str::is_empty)
            || !row["data"].is_object()
            || millis(&row["timestamp"]) == 0
        {
            return Err(invalid("事件格式或时间戳无效"));
        }
        if !seen.insert(id.to_owned()) {
            return Err(invalid("重复事件 ID"));
        }
        // A missing predecessor is permitted after the CLI's own log truncation.
        if row["parentId"].as_str() == Some(id) {
            return Err(invalid("事件引用自身"));
        }
        rows.push(row);
    }
    let start = rows.first().ok_or_else(|| invalid("空会话"))?;
    if start["type"] != "session.start" || start["data"]["version"].as_u64() != Some(1) {
        return Err(invalid("仅支持 session.start version 1 的持久化日志"));
    }
    let id = start["data"]["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("缺少 sessionId"))?;
    let expected = if path.file_name().is_some_and(|s| s == "events.jsonl") {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_stem()
    };
    if expected.and_then(|s| s.to_str()) != Some(id) {
        return Err(invalid("目录与 sessionId 不一致"));
    }
    if rows.iter().skip(1).any(|r| r["type"] == "session.start") {
        return Err(invalid("多个 session.start"));
    }
    let mut events = Vec::new();
    let mut calls = HashSet::new();
    for row in &rows {
        ensure_not_cancelled(cancel)?;
        let data = &row["data"];
        let mut content = Vec::new();
        let mut role = "meta";
        match row["type"].as_str().unwrap() {
            "user.message" => {
                role = "user";
                content.push(json!({"type":"text","text":data["content"].as_str().unwrap_or("")}));
            }
            "assistant.message" => {
                role = "assistant";
                if let Some(text) = data["content"].as_str().filter(|s| !s.is_empty()) {
                    content.push(json!({"type":"text","text":text}));
                }
                if let Some(text) = data["reasoningText"].as_str() {
                    content.push(json!({"type":"thinking","thinking":text}));
                }
                if let Some(requests) = data["toolRequests"].as_array() {
                    for call in requests {
                        if let Some(id) = call["toolCallId"].as_str() {
                            calls.insert(id.to_owned());
                        }
                        content.push(json!({"type":"tool_use","id":call["toolCallId"],"name":call["name"],"input":call["arguments"]}));
                    }
                }
            }
            "tool.execution_start" => {
                if let Some(id) = data["toolCallId"].as_str() {
                    if calls.insert(id.to_owned()) {
                        role = "assistant";
                        content.push(json!({"type":"tool_use","id":id,"name":data["toolName"],"input":data["arguments"]}));
                    }
                }
            }
            "tool.execution_complete" => {
                role = "user";
                content.push(json!({"type":"tool_result","tool_use_id":data["toolCallId"],"is_error":data["success"]==false,"content":data["result"]["content"].as_str().map(str::to_owned).unwrap_or_else(||data["error"].to_string())}));
            }
            _ => {}
        }
        // Root transcript search must not include a background agent's injected prompt.
        if row["agentId"].as_str().is_some_and(|s| !s.is_empty())
            || row["ephemeral"] == true
            || (row["type"] == "user.message"
                && (data["isAutopilotContinuation"] == true
                    || data["source"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("skill-") || s.starts_with("agent-"))))
        {
            role = "meta";
            content.clear();
        }
        events.push(ro::event(
            events.len(),
            role,
            Value::Array(content),
            millis(&row["timestamp"]),
            row.clone(),
        ));
    }
    Ok((rows, events))
}
pub fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    if !is_main_transcript(root, path) {
        return Ok(None);
    }
    let (rows, events) = read(root, path, cancel)?;
    let start = &rows[0]["data"];
    let cwd = rows
        .iter()
        .rev()
        .filter(|r| r["type"] == "session.context_changed")
        .find_map(|r| r.pointer("/data/cwd").and_then(Value::as_str))
        .or_else(|| start.pointer("/context/cwd").and_then(Value::as_str))
        .unwrap_or("");
    let title = rows
        .iter()
        .rev()
        .filter(|r| r["type"] == "session.title_changed")
        .find_map(|r| r.pointer("/data/title").and_then(Value::as_str))
        .unwrap_or("");
    let mut summary = ro::summary(
        "copilot",
        start["sessionId"].as_str().unwrap().into(),
        path.to_string_lossy().into_owned(),
        cwd.into(),
        title.into(),
        rows.iter()
            .map(|r| millis(&r["timestamp"]) / 1000)
            .min()
            .unwrap_or(0),
        rows.iter()
            .map(|r| millis(&r["timestamp"]) / 1000)
            .max()
            .unwrap_or(0),
        &events,
        fs::metadata(path)?.len(),
    );
    summary.model = rows
        .iter()
        .rev()
        .filter(|r| r["type"] == "session.model_change")
        .find_map(|r| r.pointer("/data/newModel").and_then(Value::as_str))
        .or_else(|| start["selectedModel"].as_str())
        .map(str::to_owned);
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.into());
    Ok(Some(summary))
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
        parse_session(&root_for(p)?, p, None)?.ok_or_else(|| invalid("会话已变化"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = include_str!("../tests/fixtures/copilot/copilot_stage0_small.jsonl");
    fn fixture() -> (crate::readonly_source::test_support::TempDir, PathBuf) {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let p = dir
            .path()
            .join("session-state/copilot_stage0_small/events.jsonl");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, SAMPLE).unwrap();
        (dir, p)
    }
    #[test]
    fn copilot_projection_preserves_text_and_excludes_tools_and_injected_prompts() {
        let (dir, p) = fixture();
        let before = fs::read(&p).unwrap();
        assert!(events(p.to_str().unwrap(), None).is_err());
        let summary = parse_session(dir.path(), &p, None).unwrap().unwrap();
        assert_eq!(summary.cwd, "/tmp/repo");
        assert_eq!(summary.first_user_message, "List the files");
        assert_eq!(summary.model.as_deref(), Some("gpt-5-mini"));
        let events = events(p.to_str().unwrap(), None).unwrap();
        assert_eq!(events.len(), 15);
        assert_eq!(events.iter().filter(|e| e.role == "tool_result").count(), 1);
        assert_eq!(events.iter().filter(|e| e.role == "tool_call").count(), 1);
        let text = events
            .iter()
            .filter(|e| {
                crate::rollout::preview_event_is_conversation(e)
                    || crate::rollout::preview_event_has_assistant_text_tool_use(e)
            })
            .map(crate::rollout::preview_event_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Listing the files."));
        assert!(!text.contains("a.txt"));
        assert!(!text.contains("trimmed for fixture"));
        assert_eq!(
            preview_range(p.to_str().unwrap(), 8, 1).unwrap()[0].index,
            8
        );
        assert_eq!(fs::read(&p).unwrap(), before);
        let mut injected: Value = serde_json::from_str(SAMPLE.lines().nth(6).unwrap()).unwrap();
        injected["id"] = "injected".into();
        injected["data"]["isAutopilotContinuation"] = true.into();
        injected["data"]["content"] = "autopilot should not be conversation".into();
        fs::write(&p, format!("{SAMPLE}{injected}\n")).unwrap();
        assert_eq!(
            read(dir.path(), &p, None).unwrap().1.last().unwrap().role,
            "meta"
        );
    }
    #[test]
    fn copilot_refuses_partial_duplicate_future_and_wrong_identity() {
        let (dir, p) = fixture();
        assert!(matches!(
            parse_session(dir.path(), &p, Some(&AtomicBool::new(true))),
            Err(AppError::Cancelled)
        ));
        for bad in [
            SAMPLE.trim_end().to_owned(),
            format!("{SAMPLE}{}\n", SAMPLE.lines().nth(1).unwrap()),
            SAMPLE.replacen("\"version\":1", "\"version\":99", 1),
            SAMPLE.replacen("copilot_stage0_small", "wrong-session", 1),
        ] {
            fs::write(&p, bad).unwrap();
            assert!(parse_session(dir.path(), &p, None).is_err());
        }
        let flat = dir.path().join("session-state/copilot_stage0_small.jsonl");
        fs::write(&flat, SAMPLE).unwrap();
        assert!(parse_session(dir.path(), &flat, None).unwrap().is_some());
        assert!(!is_main_transcript(
            dir.path(),
            &dir.path().join("session-state/id/subagent/events.jsonl")
        ));
    }
}
