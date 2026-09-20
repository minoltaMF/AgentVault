//! Read-only Qoder CLI transcripts. The CLI envelope is compatible with Claude's
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
            "仅支持 Qoder CLI projects/<project>/<session>.jsonl".into(),
        ));
    }
    crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "Qoder CLI transcript",
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
        "请先扫描当前 Qoder CLI 来源再预览会话".into(),
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
    session.provider = "qoder".into();
    // This adapter deliberately exposes no native execution or mutation capability.
    session.resume_command.clear();
    session.source = Some("cli".into());
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.to_path_buf());
    Ok(Some(session))
}

// Only documented dialogue blocks are exposed as conversation text.
pub(crate) fn normalize(mut raw: serde_json::Value) -> Option<serde_json::Value> {
    if raw.get("isSidechain").and_then(serde_json::Value::as_bool) == Some(true) {
        raw["isMeta"] = true.into();
    }
    if let Some(content) = raw.get_mut("message").and_then(|m| m.get_mut("content")) {
        if let Some(blocks) = content.as_array_mut() {
            blocks.retain_mut(|block| {
                match block.get("type").and_then(serde_json::Value::as_str) {
                    Some("output_text") => {
                        block["type"] = "text".into();
                        true
                    }
                    Some("text" | "thinking" | "tool_use" | "tool_result") => true,
                    _ => false,
                }
            });
        } else if !content.is_string() {
            *content = serde_json::Value::Null;
        }
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
pub(crate) fn list_sessions(
    root: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<SessionSummary>> {
    ensure_not_cancelled(cancel)?;
    let projects = root.join("projects");
    crate::path_safety::validate_descendant(
        root,
        &projects,
        crate::path_safety::EntryKind::Directory,
        false,
        "Qoder projects",
    )?;
    let mut sessions = Vec::new();
    for project in std::fs::read_dir(projects)? {
        ensure_not_cancelled(cancel)?;
        let project = project?.path();
        let meta = std::fs::symlink_metadata(&project)?;
        if !meta.is_dir() {
            continue;
        }
        crate::path_safety::validate_descendant(
            root,
            &project,
            crate::path_safety::EntryKind::Directory,
            false,
            "Qoder project",
        )?;
        for entry in std::fs::read_dir(project)? {
            ensure_not_cancelled(cancel)?;
            let path = entry?.path();
            if !is_main_transcript(root, &path) {
                continue;
            }
            if let Some(session) = parse_session(root, &path, cancel)? {
                sessions.push(session);
            }
        }
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qoder_normalizes_documented_blocks_and_excludes_internal_dialogue() {
        let raw = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"output_text","text":"visible"}, {"type":"unknown","text":"hidden"},
            {"type":"tool_use","id":"tool1","name":"shell","input":{"command":"secret"}}
        ]}});
        let event = classify_preview(7, raw.clone()).unwrap();
        assert_eq!(event.index, 7);
        assert_eq!(crate::rollout::preview_event_text(&event), "visible");
        let mut sidechain = raw;
        sidechain["isSidechain"] = true.into();
        let event = classify_preview(8, sidechain).unwrap();
        assert_eq!(event.role, "meta");
        assert!(!crate::rollout::preview_event_is_conversation(&event));
    }
    #[test]
    fn qoder_reuses_message_envelope_without_claude_capabilities() -> AppResult<()> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("agentvault-qoder-{}-{unique}", std::process::id()));
        let project = root.join("projects").join("fixture");
        std::fs::create_dir_all(&project)?;
        let path = project.join("sample.jsonl");
        std::fs::write(
            &path,
            include_str!("../tests/fixtures/qoder/sample-qoder-session.jsonl"),
        )?;
        let list = list_sessions(&root, None)?;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].provider, "qoder");
        assert!(list[0].resume_command.is_empty());
        validate_preview(path.to_str().unwrap())?;
        let events = crate::rollout::preview_session_range(
            Some("qoder".into()),
            path.to_string_lossy().into_owned(),
            0,
            100,
        )?;
        assert_eq!(events.len(), 3);
        assert_eq!(list[0].first_user_message, "你好");
        let prompts = crate::rollout::preview_session_user_prompts(
            Some("qoder".into()),
            path.to_string_lossy().into_owned(),
        )?;
        assert_eq!(prompts.prompts.len(), 1);
        let meta = crate::rollout::preview_session_meta(
            Some("qoder".into()),
            path.to_string_lossy().into_owned(),
        )?;
        assert_eq!(meta.model_provider.as_deref(), Some("qoder"));
        assert_eq!(meta.source.as_deref(), Some("cli"));
        std::fs::write(project.join("ignored-session.json"), "{}")?;
        std::fs::create_dir_all(project.join("subagents"))?;
        std::fs::write(project.join("subagents/agent.jsonl"), "{}")?;
        assert_eq!(list_sessions(&root, None)?.len(), 1);
        assert!(!is_main_transcript(
            &root,
            &project.join("subagents").join("agent.jsonl")
        ));
        assert!(validate_preview(root.join("other.jsonl").to_str().unwrap()).is_err());
        assert!(list_sessions(&root, Some(&AtomicBool::new(true))).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
