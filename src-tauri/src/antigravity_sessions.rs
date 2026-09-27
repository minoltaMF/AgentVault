//! Read-only Antigravity CLI transcripts and explicitly incomplete brain artifacts.
//! Projection adapted from agent-sessions ab439f13211e56b809f4a917d5c38e80d2bb5258 (MIT).
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
fn relative(root: &Path, path: &Path) -> Option<PathBuf> {
    for base in [
        root.to_path_buf(),
        root.join("brain"),
        root.join("antigravity/brain"),
        root.join("antigravity-cli/brain"),
        root.join("antigravity-ide/brain"),
    ] {
        let Ok(p) = path.strip_prefix(base) else {
            continue;
        };
        if !p
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let n = p.components().count();
        if (n == 2 && p.extension().and_then(|s| s.to_str()) == Some("md"))
            || (n == 4 && p.ends_with(".system_generated/logs/transcript.jsonl"))
        {
            return Some(p.into());
        }
    }
    None
}
pub fn is_main_transcript(root: &Path, path: &Path) -> bool {
    relative(root, path).is_some()
}
fn millis(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}
fn unwrap_user(text: &str) -> String {
    if let Some((_, tail)) = text.split_once("<USER_REQUEST>") {
        if let Some((s, _)) = tail.split_once("</USER_REQUEST>") {
            return s.trim().into();
        }
    }
    text.split("<ADDITIONAL_METADATA>")
        .next()
        .unwrap_or("")
        .split("<USER_SETTINGS_CHANGE>")
        .next()
        .unwrap_or("")
        .trim()
        .into()
}
fn read(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<(Vec<PreviewEvent>, SessionSummary)> {
    ensure_not_cancelled(cancel)?;
    let _relative = relative(root, path)
        .ok_or_else(|| AppError::Path("仅支持 Antigravity brain 产物及 CLI transcript".into()))?;
    ro::validate_file(root, path)?;
    let metadata = fs::metadata(path)?;
    let limit = 50 * 1024 * 1024u64;
    let mut reader = fs::File::open(path)?.take(limit + 1);
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
    if bytes.len() as u64 > limit {
        return Err(AppError::Other("Antigravity 文件超过 50 MiB".into()));
    }
    let text = String::from_utf8(bytes).map_err(|e| AppError::Other(e.to_string()))?;
    let fallback = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let artifact = path.extension().and_then(|s| s.to_str()) == Some("md");
    let mut out = Vec::new();
    let mut first = 0;
    let mut last = 0;
    let mut title = String::new();
    let id = if artifact {
        format!(
            "artifact:{}",
            path.strip_prefix(root).unwrap().to_string_lossy()
        )
    } else {
        format!(
            "transcript:{}",
            path.strip_prefix(root).unwrap().to_string_lossy()
        )
    };
    if artifact {
        if text.trim().is_empty() {
            return Err(AppError::Other("Antigravity 产物为空".into()));
        }
        title = format!(
            "[产物·非完整对话] {}",
            text.lines()
                .find_map(|l| l
                    .trim()
                    .strip_prefix('#')
                    .map(|s| s.trim_start_matches('#').trim()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("artifact"))
        );
        out.push(ro::event(0,"meta",json!([{"type":"text","text":"此文件是 Antigravity 生成的 Markdown 产物，不是完整对话历史。"}]),fallback*1000,Value::Null));
        out.push(ro::event(
            1,
            "assistant",
            json!([{"type":"text","text":text}]),
            fallback * 1000,
            json!({"artifact":true,"incomplete":true}),
        ));
    } else {
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(AppError::Other(
                "Antigravity transcript 末行尚未写完，请稍后刷新".into(),
            ));
        }
        for (line_index, line) in text.lines().enumerate() {
            ensure_not_cancelled(cancel)?;
            if line.trim().is_empty() {
                continue;
            }
            let row: Value = serde_json::from_str(line)?;
            let kind = row["type"]
                .as_str()
                .ok_or_else(|| AppError::Other("Antigravity 记录缺少 type".into()))?;
            if !row["step_index"].is_number() {
                return Err(AppError::Other(
                    "不是 Antigravity CLI transcript：缺少 step_index".into(),
                ));
            }
            let time = millis(&row["created_at"]);
            if time > 0 {
                if first == 0 {
                    first = time;
                }
                last = last.max(time);
            }
            let mut content = row["content"].as_str().unwrap_or("").to_owned();
            let clipped = row["truncated_fields"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "content"));
            let (role, blocks) = match kind {
                "USER_INPUT" => {
                    content = unwrap_user(&content);
                    if title.is_empty() {
                        title = content
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(100)
                            .collect();
                    }
                    if clipped {
                        content.push_str("\n\n[content truncated by Antigravity CLI]");
                    }
                    ("user", json!([{"type":"text","text":content}]))
                }
                "PLANNER_RESPONSE" => {
                    let mut blocks = Vec::new();
                    if !content.is_empty() {
                        if clipped {
                            content.push_str("\n\n[content truncated by Antigravity CLI]");
                        }
                        blocks.push(json!({"type":"text","text":content}));
                    }
                    if let Some(thinking) = row["thinking"].as_str() {
                        blocks.push(json!({"type":"thinking","thinking":thinking}));
                    }
                    if let Some(calls) = row["tool_calls"].as_array() {
                        for (i, c) in calls.iter().enumerate() {
                            blocks.push(json!({"type":"tool_use","id":format!("{line_index}-{i}"),"name":c["name"],"input":c["args"]}));
                        }
                    }
                    ("assistant", Value::Array(blocks))
                }
                "RUN_COMMAND" | "VIEW_FILE" | "LIST_DIRECTORY" | "CODE_ACTION" | "SEARCH_WEB" => {
                    if clipped {
                        content.push_str("\n\n[content truncated by Antigravity CLI]");
                    }
                    (
                        "user",
                        json!([{"type":"tool_result","tool_use_id":format!("step-{}",row["step_index"]),"content":content,"is_error":row["status"]=="ERROR"}]),
                    )
                }
                _ => ("meta", json!([{"type":"text","text":content}])),
            };
            // Split blocks so a mixed assistant/tool message remains searchable as dialogue.
            for block in blocks.as_array().into_iter().flatten() {
                out.push(ro::event(
                    out.len(),
                    role,
                    json!([block]),
                    time,
                    json!({"line":line_index,"type":kind,"step_index":row["step_index"]}),
                ));
            }
        }
        if out.is_empty() {
            return Err(AppError::Other("Antigravity transcript 为空".into()));
        }
    }
    let mut summary = ro::summary(
        "antigravity",
        id,
        path.to_string_lossy().into_owned(),
        String::new(),
        title,
        if first > 0 { first / 1000 } else { fallback },
        if last > 0 { last / 1000 } else { fallback },
        &out,
        metadata.len(),
    );
    summary.source = Some(
        if artifact {
            "artifact-incomplete"
        } else {
            "transcript"
        }
        .into(),
    );
    Ok((out, summary))
}
pub fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    if !is_main_transcript(root, path) {
        return Ok(None);
    }
    let s = read(root, path, cancel)?.1;
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
        .ok_or_else(|| AppError::Path("请先扫描 Antigravity 来源".into()))
}
pub fn events(path: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let p = Path::new(path);
    Ok(read(&root_for(p)?, p, cancel)?.0)
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
    Ok(ro::meta(read(&root_for(p)?, p, None)?.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::readonly_source::test_support::TempDir, PathBuf) {
        let t = crate::readonly_source::test_support::tempdir().unwrap();
        let p = t
            .path()
            .join("antigravity-cli/brain/demo/.system_generated/logs/transcript.jsonl");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(
            &p,
            include_str!("../tests/fixtures/antigravity/cli_small.jsonl"),
        )
        .unwrap();
        (t, p)
    }
    #[test]
    fn antigravity_upstream_transcript_keeps_tools_and_clipping() {
        let (t, p) = fixture();
        let s = parse_session(t.path(), &p, None).unwrap().unwrap();
        assert_eq!(s.first_user_message, "List the files.");
        assert_eq!(s.source.as_deref(), Some("transcript"));
        let e = events(p.to_str().unwrap(), None).unwrap();
        assert!(e.iter().any(|e| e.role == "tool_result"));
        assert!(e
            .iter()
            .any(|e| crate::rollout::preview_event_text(e).contains("content truncated")));
        assert!(!s.first_user_message.contains("USER_SETTINGS_CHANGE"));
        assert!(e.iter().enumerate().all(|(i, e)| e.index == i));
    }
    #[test]
    fn antigravity_artifacts_are_separate_incomplete_entries() {
        let (t, p) = fixture();
        let artifact = t.path().join("antigravity/brain/demo/task.md");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(
            &artifact,
            include_str!("../tests/fixtures/antigravity/small.md"),
        )
        .unwrap();
        let s = parse_session(t.path(), &artifact, None).unwrap().unwrap();
        let transcript = parse_session(t.path(), &p, None).unwrap().unwrap();
        assert_ne!(s.id, transcript.id);
        assert_eq!(s.source.as_deref(), Some("artifact-incomplete"));
        assert!(s.title.contains("非完整对话"));
        let e = events(artifact.to_str().unwrap(), None).unwrap();
        assert_eq!(e[0].role, "meta");
        assert!(crate::rollout::preview_event_text(&e[1]).contains('#'));
    }
    #[test]
    fn antigravity_refuses_foreign_json_partial_tail_and_cancellation() {
        let (t, p) = fixture();
        assert!(events(p.to_str().unwrap(), None).is_err());
        fs::write(&p, "{\"type\":\"user\",\"content\":\"foreign\"}\n").unwrap();
        assert!(parse_session(t.path(), &p, None).is_err());
        fs::write(&p, "{\"type\":\"USER_INPUT\",\"step_index\":0}").unwrap();
        assert!(parse_session(t.path(), &p, None).is_err());
        assert!(matches!(
            parse_session(t.path(), &p, Some(&AtomicBool::new(true))),
            Err(AppError::Cancelled)
        ));
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    #[test]
    fn supports_all_official_brain_roots_and_preserves_unknown_events_as_meta() {
        let t = crate::readonly_source::test_support::tempdir().unwrap();
        for surface in ["antigravity", "antigravity-cli", "antigravity-ide"] {
            let p = t
                .path()
                .join(surface)
                .join("brain/same/.system_generated/logs/transcript.jsonl");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(
                &p,
                include_str!("../tests/fixtures/antigravity/cli_schema_drift.jsonl"),
            )
            .unwrap();
            let summary = parse_session(t.path(), &p, None).unwrap().unwrap();
            assert!(summary.id.contains(surface));
            let events = events(p.to_str().unwrap(), None).unwrap();
            assert_eq!(events.last().unwrap().role, "meta");
            assert_eq!(summary.first_user_message, "Go");
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_artifacts() {
        let t = crate::readonly_source::test_support::tempdir().unwrap();
        let target = t.path().join("foreign.md");
        fs::write(&target, "secret").unwrap();
        let p = t.path().join("antigravity/brain/session/task.md");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &p).unwrap();
        assert!(parse_session(t.path(), &p, None).is_err());
    }
}
