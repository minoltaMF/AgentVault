//! Read-only Qwen transcript projection. Provenance is recorded in tests/fixtures/qwen.
use crate::{
    error::{ensure_not_cancelled, AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
    readonly_source as ro,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};

static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
pub(crate) fn register_source(root: &Path, path: &Path) -> AppResult<()> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path("不是受支持的会话路径".into()));
    }
    ro::validate_file(root, path)?;
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.into());
    Ok(())
}
#[cfg(test)]
pub(crate) fn forget_source_for_test(root: &Path) {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(root);
}

pub fn is_main_transcript(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root.join("projects")) else {
        return false;
    };
    let parts: Vec<_> = relative.components().collect();
    if !parts.iter().all(|c| matches!(c, Component::Normal(_)))
        || !matches!(parts.len(), 3 | 4)
        || parts[1].as_os_str() != "chats"
        || (parts.len() == 4 && parts[2].as_os_str() != "archive")
    {
        return false;
    }
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    path.extension().and_then(|s| s.to_str()) == Some("jsonl")
        && (32..=36).contains(&id.len())
        && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
}
fn root_for(path: &Path) -> AppResult<PathBuf> {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|r| is_main_transcript(r, path))
        .cloned()
        .ok_or_else(|| AppError::Path("请先扫描 Qwen Code 来源".into()))
}
fn invalid(message: &str) -> AppError {
    AppError::Other(format!("Qwen Code: {message}"))
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
        return Err(AppError::Path(
            "仅支持 Qwen projects/*/chats 下的会话".into(),
        ));
    }
    ro::validate_file(root, path)?;
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap();
    let mut rows = Vec::<Value>::new();
    let mut reader = BufReader::new(fs::File::open(path)?.take(128 * 1024 * 1024 + 1));
    let mut line = String::new();
    let mut size = 0;
    loop {
        ensure_not_cancelled(cancel)?;
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            break;
        }
        size += count;
        if size > 128 * 1024 * 1024 {
            return Err(invalid("会话超过 128 MiB 安全读取上限"));
        }
        if !line.ends_with('\n') {
            return Err(invalid("会话尾行尚未写完，请稍后刷新"));
        }
        if line.trim().is_empty() {
            continue;
        }
        // The writer can concatenate complete objects on one physical line. Serde's
        // stream accepts exactly that shape and rejects any intervening/trailing junk.
        for row in serde_json::Deserializer::from_str(&line).into_iter::<Value>() {
            ensure_not_cancelled(cancel)?;
            let row = row?;
            if row["sessionId"]
                .as_str()
                .is_none_or(|s| !s.eq_ignore_ascii_case(id))
                || row["uuid"].as_str().is_none_or(str::is_empty)
                || !(row["parentUuid"].is_string() || row.get("parentUuid") == Some(&Value::Null))
            {
                return Err(invalid("会话身份或父节点无效"));
            }
            if !matches!(
                row["type"].as_str(),
                Some("user" | "assistant" | "tool_result" | "system")
            ) {
                return Err(invalid("不支持的记录类型，请更新适配器"));
            }
            if row["type"] == "system"
                && matches!(
                    row["subtype"].as_str(),
                    Some(
                        "session_artifact_event"
                            | "session_artifact_snapshot"
                            | "session_sources_snapshot"
                            | "managed_session_header_v1"
                            | "managed_session_event_v1"
                            | "managed_session_commit_v1"
                    )
                )
            {
                continue;
            }
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(invalid("未找到有效会话记录"));
    }
    let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();
    for row in &rows {
        let fragments = grouped
            .entry(row["uuid"].as_str().unwrap().into())
            .or_default();
        if fragments.first().is_some_and(|first| {
            first["parentUuid"] != row["parentUuid"] || first["type"] != row["type"]
        }) {
            return Err(invalid("重复记录的类型或父节点冲突"));
        }
        fragments.push(row.clone());
    }
    let mut current = rows.last().unwrap()["uuid"].as_str().unwrap().to_owned();
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    while !current.is_empty() {
        ensure_not_cancelled(cancel)?;
        if !visited.insert(current.clone()) {
            return Err(invalid("会话父节点存在循环"));
        }
        let fragments = grouped
            .get(&current)
            .ok_or_else(|| invalid("会话父节点缺失"))?;
        let mut merged = fragments[0].clone();
        let mut parts = Vec::new();
        for fragment in fragments {
            if let Some(p) = fragment.pointer("/message/parts").and_then(Value::as_array) {
                parts.extend(p.iter().cloned());
            }
            if millis(&fragment["timestamp"]) > millis(&merged["timestamp"]) {
                merged["timestamp"] = fragment["timestamp"].clone();
            }
            if merged["model"].is_null() && !fragment["model"].is_null() {
                merged["model"] = fragment["model"].clone();
            }
            if merged["toolCallResult"].is_null() && !fragment["toolCallResult"].is_null() {
                merged["toolCallResult"] = fragment["toolCallResult"].clone();
            }
        }
        if !merged["message"].is_object() {
            merged["message"] = json!({});
        }
        merged["message"]["parts"] = Value::Array(parts);
        current = merged["parentUuid"].as_str().unwrap_or("").into();
        chain.push(merged);
    }
    chain.reverse();
    let mut events = Vec::new();
    for row in &chain {
        ensure_not_cancelled(cancel)?;
        let kind = row["type"].as_str().unwrap();
        let mut content = Vec::new();
        let mut role = kind;
        let parts = row["message"]["parts"].as_array().unwrap();
        if kind == "system"
            || (kind == "user"
                && matches!(
                    row["subtype"].as_str(),
                    Some("goal_runtime" | "cron" | "notification")
                ))
        {
            role = "meta";
        } else if kind == "user"
            && row
                .pointer("/systemPayload/displayText")
                .is_some_and(Value::is_string)
        {
            content.push(json!({"type":"text","text":row["systemPayload"]["displayText"]}));
        } else {
            for (index, part) in parts.iter().enumerate() {
                if let Some(text) = part["text"].as_str() {
                    if kind == "user"
                        && index + 1 == parts.len()
                        && parts.len() > 1
                        && row["systemPayload"].is_null()
                        && hook_context(text)
                    {
                        continue;
                    }
                    content.push(if part["thought"] == true {
                        json!({"type":"thinking","thinking":text})
                    } else {
                        json!({"type":"text","text":text})
                    });
                }
                if let Some(call) = part.get("functionCall") {
                    content.push(json!({"type":"tool_use","id":call["id"],"name":call["name"],"input":call["args"]}));
                }
                if let Some(result) = part.get("functionResponse") {
                    content.push(json!({"type":"tool_result","tool_use_id":result["id"],"content":result["response"].as_str().map(str::to_owned).unwrap_or_else(||result["response"].to_string())}));
                }
            }
            if kind == "tool_result" {
                role = "user";
                // All tool result material stays a tool block, including text fallbacks.
                content.retain(|p| p["type"] == "tool_result");
                if content.is_empty() {
                    content.push(json!({"type":"tool_result","tool_use_id":row["toolCallResult"]["callId"],"content":row["toolCallResult"]["resultDisplay"].to_string()}));
                }
            }
        }
        events.push(ro::event(
            events.len(),
            role,
            Value::Array(content),
            millis(&row["timestamp"]),
            row.clone(),
        ));
    }
    Ok((chain, events))
}
fn hook_context(text: &str) -> bool {
    let text = text.trim();
    let open = "<qwen:user-prompt-submit-context>\n";
    let close = "\n</qwen:user-prompt-submit-context>";
    if !text.starts_with(open) || !text.ends_with(close) {
        return false;
    }
    if text.len() <= open.len() + close.len() {
        return true;
    }
    let body = &text[open.len()..text.len() - close.len()];
    !body.contains("<qwen:user-prompt-submit-context>")
        && !body.contains("</qwen:user-prompt-submit-context>")
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
    let cwd = rows
        .iter()
        .rev()
        .find_map(|r| r["cwd"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("");
    let title = rows
        .iter()
        .rev()
        .filter(|r| r["type"] == "system" && r["subtype"] == "custom_title")
        .find_map(|r| {
            r.pointer("/systemPayload/customTitle")
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    let mut summary = ro::summary(
        "qwen",
        path.file_stem().unwrap().to_string_lossy().into_owned(),
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
        .find_map(|r| r["model"].as_str())
        .map(str::to_owned);
    summary.archived = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|s| s == "archive");
    register_source(root, path)?;
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
    const ID: &str = "019f0000-0000-7000-8000-000000000001";
    const SAMPLE: &str =
        include_str!("../tests/fixtures/qwen/019f0000-0000-7000-8000-000000000001.jsonl");
    fn fixture() -> (crate::readonly_source::test_support::TempDir, PathBuf) {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let path = dir
            .path()
            .join(format!("projects/project/chats/{ID}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, SAMPLE).unwrap();
        (dir, path)
    }
    #[test]
    fn qwen_active_branch_fragments_hook_context_and_readonly() {
        let (dir, p) = fixture();
        let before = fs::read(&p).unwrap();
        assert!(events(p.to_str().unwrap(), None).is_err());
        let summary = parse_session(dir.path(), &p, None).unwrap().unwrap();
        assert_eq!(summary.title, "Qwen synthetic branch");
        assert_eq!(summary.first_user_message, "visible question");
        let events = events(p.to_str().unwrap(), None).unwrap();
        assert_eq!(events.len(), 4);
        let text = events
            .iter()
            .filter(|e| {
                crate::rollout::preview_event_is_conversation(e)
                    || crate::rollout::preview_event_has_assistant_text_tool_use(e)
            })
            .map(crate::rollout::preview_event_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("active answer"));
        assert!(text.contains("fragment tail"));
        for absent in ["dead branch", "hidden hook", "secret-tool"] {
            assert!(!text.contains(absent), "{text}");
        }
        assert_eq!(
            preview_range(p.to_str().unwrap(), 1, 1).unwrap()[0].index,
            1
        );
        assert_eq!(fs::read(&p).unwrap(), before);
    }
    #[test]
    fn qwen_non_conversation_tail_does_not_replace_active_branch() {
        let (dir, p) = fixture();
        let baseline = read(dir.path(), &p, None).unwrap().1;
        // Official isTranscriptConversationRecord excludes all six system subtypes.
        // They may carry independent ancestry and must never become the active leaf.
        for subtype in [
            "session_artifact_event",
            "session_artifact_snapshot",
            "session_sources_snapshot",
            "managed_session_header_v1",
            "managed_session_event_v1",
            "managed_session_commit_v1",
        ] {
            for parent in [Value::Null, json!("dead")] {
                let tail = json!({
                    "uuid":"non-conversation-tail", "parentUuid":parent,
                    "sessionId":ID, "type":"system", "subtype":subtype,
                    "timestamp":"2026-09-24T01:00:00Z",
                    "message":{"role":"user","parts":[]}
                });
                fs::write(&p, format!("{SAMPLE}{tail}\n")).unwrap();
                let events = read(dir.path(), &p, None).unwrap().1;
                assert_eq!(
                    serde_json::to_value(&events).unwrap(),
                    serde_json::to_value(&baseline).unwrap(),
                    "non-conversation subtype {subtype} changed the selected branch"
                );
            }
        }
    }
    #[test]
    fn qwen_rejects_broken_graph_identity_and_partial_tail() {
        let (dir, p) = fixture();
        assert!(matches!(
            parse_session(dir.path(), &p, Some(&AtomicBool::new(true))),
            Err(AppError::Cancelled)
        ));
        for bad in [
            SAMPLE.trim_end().to_owned(),
            SAMPLE.replace("\"parentUuid\":null", "\"parentUuid\":\"title\""),
            SAMPLE.replace("\"parentUuid\":\"tool\"", "\"parentUuid\":\"missing\""),
            SAMPLE.replacen(ID, "019f0000-0000-7000-8000-000000000002", 1),
            SAMPLE.replacen("\"type\":\"user\"", "\"type\":\"future-message\"", 1),
        ] {
            fs::write(&p, bad).unwrap();
            assert!(parse_session(dir.path(), &p, None).is_err());
        }
    }
    #[test]
    fn qwen_recovers_concatenated_objects_and_limits_discovery() {
        let (dir, p) = fixture();
        fs::write(&p, format!("{}\n", SAMPLE.replace('\n', ""))).unwrap();
        assert!(parse_session(dir.path(), &p, None).unwrap().is_some());
        fs::write(&p, format!("{}junk\n", SAMPLE.trim_end())).unwrap();
        assert!(parse_session(dir.path(), &p, None).is_err());
        let archived = dir
            .path()
            .join(format!("projects/project/chats/archive/{ID}.jsonl"));
        fs::create_dir_all(archived.parent().unwrap()).unwrap();
        fs::write(&archived, SAMPLE).unwrap();
        assert!(
            parse_session(dir.path(), &archived, None)
                .unwrap()
                .unwrap()
                .archived
        );
        assert!(!is_main_transcript(
            dir.path(),
            &dir.path().join("projects/project/chats/telemetry.jsonl")
        ));
        assert!(!is_main_transcript(
            dir.path(),
            &dir.path()
                .join(format!("projects/project/backup/chats/{ID}.jsonl"))
        ));
    }
}
