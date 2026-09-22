//! Read-only Grok Build CLI sessions. Wire format and rewind tracking adapted from
//! xai-org/grok-build 4247f661689354b831191f11eeeac8424993fe3d (Apache-2.0).
//! Copyright 2023-2026 SpaceXAI. Modified for AgentVault: no native writes/runtime.
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex, OnceLock};

use serde_json::{json, Value};

use crate::error::{ensure_not_cancelled, AppError, AppResult};
use crate::models::{PreviewEvent, SessionSummary};

static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub(crate) fn is_main_transcript(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root.join("sessions"))
        .ok()
        .is_some_and(|p| {
            p.components().count() == 3
                && p.components().all(|c| matches!(c, Component::Normal(_)))
                && p.file_name().and_then(|n| n.to_str()) == Some("updates.jsonl")
        })
}

fn metadata(root: &Path, path: &Path) -> AppResult<Value> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path(
            "仅支持 Grok sessions/<project>/<id>/updates.jsonl".into(),
        ));
    }
    let summary = path.with_file_name("summary.json");
    for candidate in [path, summary.as_path()] {
        crate::path_safety::validate_descendant(
            root,
            candidate,
            crate::path_safety::EntryKind::File,
            false,
            "Grok CLI session",
        )?;
    }
    let value: Value = serde_json::from_reader(std::fs::File::open(summary)?)?;
    let id = value
        .pointer("/info/id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if id.is_empty()
        || value.pointer("/info/cwd").and_then(Value::as_str).is_none()
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some(id)
    {
        return Err(AppError::Other(
            "Grok summary 缺少有效 info.id/cwd 或会话目录不匹配".into(),
        ));
    }
    Ok(value)
}

fn hidden(meta: &Value) -> bool {
    meta.get("hidden")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            meta.get("session_kind")
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with("subagent"))
        })
}

pub(crate) fn validate_preview(path: &str) -> AppResult<()> {
    let roots = ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for root in roots.iter() {
        if is_main_transcript(root, Path::new(path)) {
            if hidden(&metadata(root, Path::new(path))?) {
                return Err(AppError::Path(
                    "Grok 隐藏或子 Agent 会话不在当前只读接入范围".into(),
                ));
            }
            return Ok(());
        }
    }
    Err(AppError::Path(
        "请先扫描当前 Grok CLI 来源再预览会话".into(),
    ))
}

fn seconds(value: Option<&Value>) -> i64 {
    value
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp())
        .unwrap_or(0)
}

pub(crate) fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    ensure_not_cancelled(cancel)?;
    let meta = metadata(root, path)?;
    if hidden(&meta) {
        return Ok(None);
    }
    let events = read_events(path, cancel)?;
    let first_user_message = events
        .iter()
        .find(|e| e.role == "user")
        .map(crate::rollout::preview_event_text)
        .unwrap_or_default();
    let title = meta
        .get("generated_title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta.get("session_summary").and_then(Value::as_str))
        .unwrap_or(&first_user_message);
    let cwd = meta
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .unwrap_or("");
    let session = SessionSummary {
        provider: "grok".into(),
        id: meta["info"]["id"].as_str().unwrap().into(),
        rollout_path: path.to_string_lossy().into_owned(),
        cwd: crate::paths::strip_verbatim(cwd),
        cwd_display: crate::paths::basename_display(cwd),
        title: title.chars().take(160).collect(),
        first_user_message,
        model: meta
            .get("current_model_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        reasoning_effort: meta
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        source: Some("cli".into()),
        agent_nickname: None,
        agent_role: None,
        conversion_origin: None,
        tokens_used: 0,
        created_at: seconds(meta.get("created_at")),
        updated_at: seconds(
            meta.get("last_active_at")
                .filter(|v| v.is_string())
                .or_else(|| meta.get("updated_at")),
        ),
        archived: false,
        git_branch: meta
            .get("head_branch")
            .and_then(Value::as_str)
            .map(str::to_owned),
        rollout_bytes: std::fs::metadata(path)?.len(),
        logs_count: 0,
        has_backup: false,
        resume_command: String::new(),
    };
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.to_path_buf());
    Ok(Some(session))
}

struct Row {
    index: usize,
    timestamp: String,
    update: Value,
    extension: bool,
}

// Adapted from upstream UserRunTurnTracker/filter_rewind_by: once indexed turns
// appear, unindexed host echoes do not open prompt boundaries.
#[derive(Default)]
struct Turns {
    seen_marker: bool,
    in_user: bool,
    current: Option<u64>,
}
impl Turns {
    fn user(&mut self, index: Option<u64>) -> bool {
        self.seen_marker |= index.is_some();
        let counts = !self.seen_marker || index.is_some();
        let new = !self.in_user || (self.seen_marker && index != self.current);
        self.current = index;
        self.in_user = true;
        new && counts
    }
    fn boundary(&mut self) {
        self.in_user = false;
        self.current = None;
    }
}

fn read_events(path: &Path, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let mut rows: Vec<Row> = Vec::new();
    let mut starts = Vec::new();
    let mut turns = Turns::default();
    for (index, line) in std::io::BufReader::new(std::fs::File::open(path)?)
        .lines()
        .enumerate()
    {
        ensure_not_cancelled(cancel)?;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|e| {
            AppError::Other(format!("Grok updates 第 {} 行 JSON 无效: {e}", index + 1))
        })?;
        let extension = value.get("method").and_then(Value::as_str) == Some("_x.ai/session/update");
        let params = value.get("params").unwrap_or(&value);
        let Some(update) = params.get("update") else {
            turns.boundary();
            continue;
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        if extension && kind == "rewind_marker" {
            let target = update
                .get("target_prompt_index")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| {
                    AppError::Other("Grok rewind_marker 缺少有效 target_prompt_index".into())
                })?;
            rows.truncate(starts.get(target).copied().unwrap_or(rows.len()));
            starts.truncate(target);
            turns.boundary();
            continue;
        }
        if !extension
            && kind == "user_message_chunk"
            && update.pointer("/_meta/hostTurn").and_then(Value::as_bool) != Some(true)
        {
            if turns.user(update.pointer("/_meta/promptIndex").and_then(Value::as_u64)) {
                starts.push(rows.len());
            }
        } else {
            turns.boundary();
        }
        let millis = params
            .pointer("/_meta/agentTimestampMs")
            .and_then(Value::as_i64);
        let timestamp = millis
            .and_then(chrono::DateTime::from_timestamp_millis)
            .or_else(|| {
                value
                    .get("timestamp")
                    .and_then(Value::as_i64)
                    .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            })
            .map(|t| t.to_rfc3339())
            .unwrap_or_default();
        rows.push(Row {
            index,
            timestamp,
            update: update.clone(),
            extension,
        });
    }
    normalize_rows(rows, cancel)
}

fn message(row: &Row, role: &str, block: Value) -> PreviewEvent {
    crate::claude_sessions::classify_preview(
        row.index,
        json!({
            "type":role, "timestamp":row.timestamp, "message":{"role":role,"content":[block]}
        }),
    )
    .expect("Claude classifier always emits normalized messages")
}

fn normalize_rows(rows: Vec<Row>, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let mut out: Vec<PreviewEvent> = Vec::new();
    let mut previous: Option<(String, Option<u64>)> = None;
    let mut tools: HashMap<String, usize> = HashMap::new();
    for row in rows {
        ensure_not_cancelled(cancel)?;
        let kind = row
            .update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        if row.extension {
            previous = None;
            continue;
        }
        if matches!(
            kind,
            "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk"
        ) {
            if row
                .update
                .pointer("/_meta/hostTurn")
                .and_then(Value::as_bool)
                == Some(true)
                || row.update.pointer("/content/_meta/bash_command").is_some()
            {
                previous = None;
                continue;
            }
            let Some(text) = row
                .update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .filter(|_| {
                    row.update.pointer("/content/type").and_then(Value::as_str) == Some("text")
                })
            else {
                previous = None;
                continue;
            };
            let pi = row
                .update
                .pointer("/_meta/promptIndex")
                .and_then(Value::as_u64);
            let key = (kind.to_string(), pi);
            let thinking = kind == "agent_thought_chunk";
            let field = if thinking { "thinking" } else { "text" };
            if previous.as_ref() == Some(&key) {
                if let Some(last) = out.last_mut() {
                    if let Value::String(value) = &mut last.raw["message"]["content"][0][field] {
                        value.push_str(text);
                    }
                }
            } else {
                let block = if thinking {
                    json!({"type":"thinking","thinking":text})
                } else {
                    json!({"type":"text","text":text})
                };
                out.push(message(
                    &row,
                    if kind == "user_message_chunk" {
                        "user"
                    } else {
                        "assistant"
                    },
                    block,
                ));
            }
            previous = Some(key);
            continue;
        }
        previous = None;
        if matches!(kind, "tool_call" | "tool_call_update") {
            let Some(id) = row.update.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            let slot = if let Some(slot) = tools.get(id) {
                *slot
            } else {
                let slot = out.len();
                out.push(message(
                    &row,
                    "assistant",
                    json!({"type":"tool_use","id":id,"name":"tool","input":{}}),
                ));
                tools.insert(id.to_string(), slot);
                slot
            };
            let block = &mut out[slot].raw["message"]["content"][0];
            if let Some(title) = row.update.get("title").and_then(Value::as_str) {
                block["name"] = title.into();
            }
            if let Some(input) = row.update.get("rawInput") {
                block["input"] = input.clone();
            }
            if let Some(status) = row.update.get("status") {
                block["status"] = status.clone();
            }
            // Preserve structured tool output inside the tool block, never as body text.
            if let Some(content) = row.update.get("content") {
                block["output"] = content.clone();
            }
            if let Some(output) = row.update.get("rawOutput") {
                block["rawOutput"] = output.clone();
            }
        }
    }
    // Classify once per aggregated event, avoiding quadratic copies for token streams.
    for event in &mut out {
        ensure_not_cancelled(cancel)?;
        let raw = std::mem::take(&mut event.raw);
        *event = crate::claude_sessions::classify_preview(event.index, raw).unwrap();
    }
    Ok(out)
}

pub(crate) fn events(path: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    ensure_not_cancelled(cancel)?;
    validate_preview(path)?;
    read_events(Path::new(path), cancel)
}

pub(crate) fn preview_range(
    path: &str,
    offset: usize,
    limit: usize,
) -> AppResult<Vec<PreviewEvent>> {
    if limit == 0 {
        validate_preview(path)?;
        return Ok(Vec::new());
    }
    Ok(events(path, None)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}

pub(crate) fn preview_meta(path: &str) -> AppResult<crate::models::SessionMetaBrief> {
    validate_preview(path)?;
    let meta: Value = serde_json::from_reader(std::fs::File::open(
        Path::new(path).with_file_name("summary.json"),
    )?)?;
    Ok(crate::models::SessionMetaBrief {
        id: meta
            .pointer("/info/id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        timestamp: meta
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
        cwd: meta
            .pointer("/info/cwd")
            .and_then(Value::as_str)
            .map(str::to_owned),
        originator: Some("Grok Build CLI".into()),
        cli_version: None,
        source: Some("cli".into()),
        model_provider: Some("grok".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("agentvault-grok-{}-{unique}", std::process::id()));
            let path = root.join("sessions/project/s/updates.jsonl");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path.with_file_name("summary.json"),
                include_str!("../tests/fixtures/grok/summary.json"),
            )
            .unwrap();
            std::fs::write(&path, include_str!("../tests/fixtures/grok/updates.jsonl")).unwrap();
            Self { root, path }
        }
        fn write(&self, updates: &[Value]) {
            let text = updates
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&self.path, text).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn chunk(kind: &str, text: &str, pi: Option<u64>) -> Value {
        let mut value = json!({"timestamp":1,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":kind,"content":{"type":"text","text":text}}}});
        if let Some(pi) = pi {
            value["params"]["update"]["_meta"] = json!({"promptIndex":pi});
        }
        value
    }
    fn rewind(target: usize) -> Value {
        json!({"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"rewind_marker","target_prompt_index":target}}})
    }

    #[test]
    fn grok_fixture_aggregates_chunks_and_keeps_native_data_read_only() -> AppResult<()> {
        let f = Fixture::new();
        let before = std::fs::read(&f.path)?;
        let session = parse_session(&f.root, &f.path, None)?.unwrap();
        assert_eq!(session.provider, "grok");
        assert_eq!(session.title, "Fixture title");
        assert_eq!(session.first_user_message, "hello world");
        assert_eq!(session.created_at, 1);
        assert_eq!(session.updated_at, 3);
        assert!(session.resume_command.is_empty());
        let es = events(f.path.to_str().unwrap(), None)?;
        assert_eq!(es.len(), 4);
        assert_eq!(es[0].index, 0);
        assert_eq!(es[1].index, 2);
        assert_eq!(
            crate::rollout::preview_event_text(&es[1]),
            "hello world reply"
        );
        assert_eq!(es[2].role, "tool_call");
        assert_eq!(es[2].raw["message"]["content"][0]["status"], "completed");
        assert_eq!(es[3].role, "reasoning");
        assert!(!crate::rollout::preview_event_is_conversation(&es[2]));
        assert_eq!(preview_range(f.path.to_str().unwrap(), 1, 1)?[0].index, 2);
        assert_eq!(
            preview_meta(f.path.to_str().unwrap())?
                .model_provider
                .as_deref(),
            Some("grok")
        );
        assert_eq!(before, std::fs::read(&f.path)?);
        Ok(())
    }

    #[test]
    fn grok_rewinds_remove_all_dead_branch_content_and_preserve_source_indices() -> AppResult<()> {
        let f = Fixture::new();
        f.write(&[
            chunk("user_message_chunk", "zero", Some(0)),
            chunk("agent_message_chunk", "keep", None),
            chunk("user_message_chunk", "one", Some(1)),
            chunk("agent_message_chunk", "dead", None),
            rewind(1),
            chunk("user_message_chunk", "replacement", Some(1)),
            chunk("agent_message_chunk", "also dead", None),
            rewind(1),
            chunk("user_message_chunk", "final ", Some(1)),
            chunk("user_message_chunk", "prompt", Some(1)),
            chunk("user_message_chunk", "next prompt", Some(2)),
        ]);
        parse_session(&f.root, &f.path, None)?;
        let es = events(f.path.to_str().unwrap(), None)?;
        assert_eq!(
            es.iter().map(|e| e.index).collect::<Vec<_>>(),
            vec![0, 1, 8, 10]
        );
        assert_eq!(crate::rollout::preview_event_text(&es[2]), "final prompt");
        assert!(!es
            .iter()
            .any(|e| crate::rollout::preview_event_text(e).contains("dead")));
        f.write(&[
            chunk("user_message_chunk", "old", None),
            rewind(0),
            chunk("user_message_chunk", "fresh", None),
        ]);
        assert_eq!(read_events(&f.path, None)?[0].index, 2);
        Ok(())
    }

    #[test]
    fn grok_legacy_and_host_chunks_do_not_leak_into_body() -> AppResult<()> {
        let f = Fixture::new();
        let mut host = chunk("user_message_chunk", "host secret", None);
        host["params"]["update"]["_meta"] = json!({"hostTurn":true});
        let mut bash = chunk("user_message_chunk", "bash secret", None);
        bash["params"]["update"]["content"]["_meta"] = json!({"bash_command":"secret"});
        f.write(&[
            chunk("user_message_chunk", "start", Some(0)),
            host,
            bash,
            chunk("agent_message_chunk", "escaped \"quote\"\n你好", None)["params"].clone(),
            chunk("user_message_chunk", "remove", Some(1)),
            rewind(1),
        ]);
        let es = read_events(&f.path, None)?;
        assert_eq!(es.len(), 2);
        assert_eq!(
            crate::rollout::preview_event_text(&es[1]),
            "escaped \"quote\"\n你好"
        );
        Ok(())
    }

    #[test]
    fn grok_rejects_unscanned_hidden_corrupt_and_cancelled_sources() -> AppResult<()> {
        let f = Fixture::new();
        assert!(validate_preview(f.path.to_str().unwrap()).is_err());
        assert!(parse_session(&f.root, &f.path, Some(&AtomicBool::new(true))).is_err());
        assert!(!is_main_transcript(
            &f.root,
            &f.path.parent().unwrap().join("subagents/x/updates.jsonl")
        ));
        let summary = f.path.with_file_name("summary.json");
        let mut meta: Value = serde_json::from_slice(&std::fs::read(&summary)?)?;
        meta["session_kind"] = "subagent_fork".into();
        std::fs::write(&summary, meta.to_string())?;
        assert!(parse_session(&f.root, &f.path, None)?.is_none());
        meta["hidden"] = false.into();
        std::fs::write(&summary, meta.to_string())?;
        assert!(parse_session(&f.root, &f.path, None)?.is_some());
        std::fs::write(&f.path, "{broken\n")?;
        assert!(events(f.path.to_str().unwrap(), None).is_err());
        meta["info"]["id"] = "another".into();
        std::fs::write(&summary, meta.to_string())?;
        assert!(validate_preview(f.path.to_str().unwrap()).is_err());
        Ok(())
    }
}
