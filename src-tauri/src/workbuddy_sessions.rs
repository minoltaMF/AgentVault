//! Read-only Tencent WorkBuddy transcripts. The normalized envelope reuses Claude's
//! message classifier; discovery and permissions remain provider-specific.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex, OnceLock};

use crate::error::{ensure_not_cancelled, AppError, AppResult};
use crate::models::SessionSummary;

static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub(crate) fn is_main_transcript(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root.join("projects"))
        .ok()
        .is_some_and(|relative| {
            relative.components().count() == 2
                && relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && relative.extension().and_then(|s| s.to_str()) == Some("jsonl")
        })
}

fn validate(root: &Path, path: &Path) -> AppResult<()> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path(
            "仅支持 Tencent WorkBuddy projects/<project>/<session>.jsonl".into(),
        ));
    }
    crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "Tencent WorkBuddy transcript",
    )?;
    Ok(())
}

pub(crate) fn validate_preview(path: &str) -> AppResult<()> {
    let path = Path::new(path);
    let roots = ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for root in roots.iter() {
        if is_main_transcript(root, path) {
            return validate(root, path);
        }
    }
    Err(AppError::Path(
        "请先扫描当前 Tencent WorkBuddy 来源再预览会话".into(),
    ))
}

pub(crate) fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    ensure_not_cancelled(cancel)?;
    validate(root, path)?;
    let Some(mut session) =
        crate::claude_sessions::parse_session_normalized(path, cancel, normalize)?
    else {
        return Ok(None);
    };
    session.provider = "workbuddy".into();
    // This adapter deliberately exposes no native execution or mutation capability.
    session.resume_command.clear();
    session.source = Some("desktop".into());
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.to_path_buf());
    Ok(Some(session))
}

// WorkBuddy stores top-level messages, not Claude envelopes. Only real user queries
// and assistant output_text blocks become searchable conversation content.
pub(crate) fn normalize(mut raw: serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    if !raw.is_object() {
        return Some(json!({"type":"unknown","isMeta":true,"original":raw}));
    }
    if let Some(ms) = raw.get("timestamp").and_then(Value::as_i64) {
        raw["timestamp"] = chrono::DateTime::from_timestamp_millis(ms)
            .map(|time| Value::String(time.to_rfc3339()))
            .unwrap_or(Value::Null);
    }
    if raw.get("type").and_then(Value::as_str) != Some("message") {
        raw["isMeta"] = true.into();
        return Some(raw);
    }
    let role = raw
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let text = raw
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| {
                    let kind = block.get("type").and_then(Value::as_str);
                    (role == "user" && kind == Some("input_text"))
                        || (role == "assistant" && kind == Some("output_text"))
                })
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    let text = if role == "user" {
        // The last user_query is the user-authored portion; earlier sections contain
        // environment/context text. Never expose compaction summaries as user turns.
        if text.trim_start().starts_with("<cb_summary>") {
            String::new()
        } else {
            text.rsplit_once("<user_query>")
                .and_then(|(_, query)| {
                    query
                        .split_once("</user_query>")
                        .map(|(query, _)| query.trim().to_owned())
                })
                .unwrap_or_default()
        }
    } else {
        text
    };
    raw["type"] = role.clone().into();
    raw["message"] = json!({"role": role, "content": text});
    if text.is_empty() || !matches!(role.as_str(), "user" | "assistant") {
        raw["isMeta"] = true.into();
    }
    Some(raw)
}
pub(crate) fn classify_preview(
    index: usize,
    raw: serde_json::Value,
) -> Option<crate::models::PreviewEvent> {
    crate::claude_sessions::classify_preview(index, normalize(raw)?)
}
pub(crate) fn preview_range(
    path: &str,
    offset: usize,
    limit: usize,
) -> AppResult<Vec<crate::models::PreviewEvent>> {
    use std::io::BufRead;
    validate_preview(path)?;
    let mut events = Vec::with_capacity(limit.min(1024));
    if limit == 0 {
        return Ok(events);
    }
    let mut visible = 0;
    for (index, line) in std::io::BufReader::new(std::fs::File::open(path)?)
        .lines()
        .enumerate()
    {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw = serde_json::from_str(&line)?;
        if let Some(event) = classify_preview(index, raw) {
            if visible >= offset {
                events.push(event);
            }
            visible += 1;
            if events.len() == limit {
                break;
            }
        }
    }
    Ok(events)
}
pub(crate) fn preview_meta(path: &str) -> AppResult<crate::models::SessionMetaBrief> {
    validate_preview(path)?;
    let session =
        crate::claude_sessions::parse_session_normalized(Path::new(path), None, normalize)?
            .ok_or_else(|| AppError::Other("WorkBuddy 会话元数据为空".into()))?;
    Ok(crate::models::SessionMetaBrief {
        id: Some(session.id),
        timestamp: chrono::DateTime::from_timestamp(session.created_at, 0).map(|t| t.to_rfc3339()),
        cwd: Some(session.cwd),
        originator: Some("Tencent WorkBuddy".into()),
        cli_version: None,
        source: Some("desktop".into()),
        model_provider: Some("workbuddy".into()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn excludes_context_reasoning_tools_and_extracts_last_user_query() {
        let event = classify_preview(4, json!({"type":"message","role":"user","timestamp":1782828714330i64,
            "content":[{"type":"input_text","text":"context <user_query>old</user_query> context <user_query>actual question</user_query> tail"}]})).unwrap();
        assert_eq!(
            crate::rollout::preview_event_text(&event),
            "actual question"
        );
        assert!(!event.timestamp.is_empty());
        assert_eq!(event.index, 4);
        for text in [
            "context only",
            "<cb_summary>summary <user_query>hidden</user_query>",
        ] {
            let event=classify_preview(0,json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]})).unwrap();
            assert!(!crate::rollout::preview_event_is_conversation(&event));
        }
        let event=classify_preview(1,json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"},{"type":"reasoning","text":"hidden"},{"type":"image_blob_ref","blob_path":"outside"}]})).unwrap();
        assert_eq!(crate::rollout::preview_event_text(&event), "answer");
    }
    #[test]
    fn workbuddy_readonly_scan_metadata_preview_and_cancel() -> AppResult<()> {
        let root = std::env::temp_dir().join(format!(
            "agentvault-workbuddy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("projects/demo");
        std::fs::create_dir_all(&project)?;
        let path = project.join("fixture.jsonl");
        let body = include_str!("../tests/fixtures/workbuddy/sample.jsonl");
        std::fs::write(&path, body)?;
        assert!(validate_preview(path.to_str().unwrap()).is_err());
        let session = parse_session(&root, &path, None)?.unwrap();
        assert_eq!(session.provider, "workbuddy");
        assert!(session.resume_command.is_empty());
        assert_eq!(session.title, "WorkBuddy fixture");
        assert_eq!(session.first_user_message, "needle question");
        assert_eq!(preview_range(path.to_str().unwrap(), 1, 1)?[0].index, 1);
        assert_eq!(
            preview_meta(path.to_str().unwrap())?
                .model_provider
                .as_deref(),
            Some("workbuddy")
        );
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            parse_session(&root, &path, Some(&cancel)),
            Err(AppError::Cancelled)
        ));
        assert!(!is_main_transcript(
            &root,
            &project.join("subagents/child.jsonl")
        ));
        assert_eq!(std::fs::read_to_string(&path)?, body);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
