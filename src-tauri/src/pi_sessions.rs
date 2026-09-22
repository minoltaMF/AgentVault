//! Read-only desktop projection of the existing Pi parser and validated branch graph.
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex, OnceLock};

use provider_pi::{PiProvider, PiSession, PiSessionError};
use provider_sdk::{
    ConsistencyClass, NativeSessionKind, NativeSessionRef, ProviderContext, SessionRoot,
    SessionRootKind,
};
use serde_json::Value;

use crate::error::{ensure_not_cancelled, AppError, AppResult};
use crate::models::{PreviewEvent, SessionSummary};

static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub(crate) fn is_main_transcript(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root.join("sessions"))
        .ok()
        .is_some_and(|relative| {
            relative.components().count() > 0
                && relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && relative.extension().and_then(|s| s.to_str()) == Some("jsonl")
        })
}

fn validate(root: &Path, path: &Path) -> AppResult<()> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path(
            "仅支持 Pi agent/sessions 下的 JSONL 会话".into(),
        ));
    }
    crate::path_safety::validate_descendant(
        root,
        path,
        crate::path_safety::EntryKind::File,
        false,
        "Pi transcript",
    )?;
    Ok(())
}

fn registered_root(path: &Path) -> AppResult<PathBuf> {
    let roots = ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for root in roots.iter() {
        if is_main_transcript(root, path) {
            validate(root, path)?;
            return Ok(root.clone());
        }
    }
    Err(AppError::Path("请先扫描当前 Pi 来源再预览会话".into()))
}

pub(crate) fn validate_preview(path: &str) -> AppResult<()> {
    registered_root(Path::new(path)).map(|_| ())
}

fn read(root: &Path, path: &Path, cancel: Option<&AtomicBool>) -> AppResult<PiSession> {
    ensure_not_cancelled(cancel)?;
    validate(root, path)?;
    let mut context = ProviderContext::new(root);
    if let Some(cancel) = cancel {
        context = context.with_cancellation(cancel);
    }
    let native = NativeSessionRef::new(
        "pi",
        None,
        path,
        SessionRoot::new(
            root.join("sessions"),
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );
    let session = PiProvider
        .read_session(&context, &native)
        .map_err(|e| match e {
            PiSessionError::Cancelled => AppError::Cancelled,
            other => AppError::Other(other.to_string()),
        })?;
    // A partial active-file tail must be reported, never presented as a complete transcript.
    if let Some(tail) = &session.partial_tail {
        return Err(AppError::Other(format!(
            "Pi 会话第 {} 行尚未写完，请稍后刷新",
            tail.line_number
        )));
    }
    Ok(session)
}

fn project(
    session: &PiSession,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<PreviewEvent>> {
    let active = session
        .graph
        .active_path()
        .map_err(|e| AppError::Other(e.to_string()))?;
    let active_ids: HashSet<&str> = active.iter().map(|n| n.id.as_str()).collect();
    // The SDK retains entry ordinals, but not physical lines. Match its exact raw records
    // against a second read to preserve source indices (including blank lines), rejecting races.
    let mut source = std::io::BufReader::new(std::fs::File::open(path)?)
        .lines()
        .enumerate();
    let mut indices = HashMap::new();
    for expected in std::iter::once(session.header.raw_json.as_str())
        .chain(session.entries.iter().map(|e| e.raw_json.as_str()))
    {
        loop {
            ensure_not_cancelled(cancel)?;
            let Some((index, line)) = source.next() else {
                return Err(AppError::Other(
                    "Pi 会话读取期间发生变化，请重新扫描".into(),
                ));
            };
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if line != expected {
                return Err(AppError::Other(
                    "Pi 会话读取期间发生变化，请重新扫描".into(),
                ));
            }
            indices.insert(expected.as_ptr() as usize, index);
            break;
        }
    }
    for line in source {
        ensure_not_cancelled(cancel)?;
        if !line.1?.trim().is_empty() {
            return Err(AppError::Other(
                "Pi 会话读取期间发生变化，请重新扫描".into(),
            ));
        }
    }
    let mut events = Vec::new();
    for entry in &session.entries {
        ensure_not_cancelled(cancel)?;
        if !active_ids.contains(entry.node.id.as_str()) {
            continue;
        }
        let mut raw = entry.raw.clone();
        let role = raw
            .pointer("/message/role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if entry.entry_type != "message" || !matches!(role.as_str(), "user" | "assistant") {
            raw["isMeta"] = true.into();
        } else {
            raw["type"] = role.into();
            if let Some(blocks) = raw
                .pointer_mut("/message/content")
                .and_then(Value::as_array_mut)
            {
                blocks.retain(|block| {
                    matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("text" | "thinking")
                    )
                });
            }
        }
        let index = indices[&(entry.raw_json.as_ptr() as usize)];
        if let Some(event) = crate::claude_sessions::classify_preview(index, raw) {
            events.push(event);
        }
    }
    Ok(events)
}

fn seconds(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|v| v.timestamp())
}

pub(crate) fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    let parsed = read(root, path, cancel)?;
    let events = project(&parsed, path, cancel)?;
    let first = events
        .iter()
        .find(|e| e.role == "user")
        .map(crate::rollout::preview_event_text)
        .unwrap_or_default();
    let title = parsed
        .entries
        .iter()
        .rev()
        .find(|e| e.entry_type == "session_info")
        .and_then(|e| e.raw.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| first.chars().take(100).collect());
    let model = events
        .iter()
        .rev()
        .find_map(|e| {
            e.raw
                .get("modelId")
                .or_else(|| e.raw.pointer("/message/model"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned);
    let cwd = parsed.header.cwd.clone().unwrap_or_default();
    let created = seconds(parsed.header.timestamp.as_deref()).unwrap_or(0);
    let updated = parsed
        .entries
        .iter()
        .filter_map(|e| seconds(e.timestamp.as_deref()))
        .max()
        .unwrap_or(created);
    let summary = SessionSummary {
        provider: "pi".into(),
        id: parsed.header.id,
        rollout_path: path.to_string_lossy().into_owned(),
        cwd_display: crate::paths::basename_display(&cwd),
        cwd,
        title,
        first_user_message: first,
        model,
        reasoning_effort: None,
        source: Some("cli".into()),
        agent_nickname: None,
        agent_role: None,
        conversion_origin: None,
        tokens_used: 0,
        created_at: created,
        updated_at: updated,
        archived: false,
        git_branch: None,
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
    Ok(Some(summary))
}

pub(crate) fn events(path: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let path = Path::new(path);
    let root = registered_root(path)?;
    let session = read(&root, path, cancel)?;
    project(&session, path, cancel)
}

pub(crate) fn preview_range(
    path: &str,
    offset: usize,
    limit: usize,
) -> AppResult<Vec<PreviewEvent>> {
    validate_preview(path)?;
    if limit == 0 {
        return Ok(Vec::new());
    }
    Ok(events(path, None)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}

pub(crate) fn preview_meta(path: &str) -> AppResult<crate::models::SessionMetaBrief> {
    let source = Path::new(path);
    let root = registered_root(source)?;
    let session = read(&root, source, None)?;
    Ok(crate::models::SessionMetaBrief {
        id: Some(session.header.id),
        timestamp: session.header.timestamp,
        cwd: session.header.cwd,
        originator: Some("pi".into()),
        cli_version: None,
        source: Some("cli".into()),
        model_provider: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = concat!(
        "\n{\"type\":\"session\",\"version\":3,\"id\":\"sample\",\"cwd\":\"/fixture\",\"timestamp\":\"2026-09-21T00:00:00Z\"}\n",
        "{\"type\":\"message\",\"id\":\"root\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"active question\"}}\n\n",
        "{\"type\":\"message\",\"id\":\"old\",\"parentId\":\"root\",\"message\":{\"role\":\"assistant\",\"content\":\"obsolete answer\"}}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"parentId\":\"root\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"current answer\"},{\"type\":\"image\",\"data\":\"hidden\"}]}}\n",
        "{\"type\":\"session_info\",\"id\":\"name\",\"parentId\":\"new\",\"name\":\"Pi fixture\"}\n"
    );
    struct Fixture {
        root: PathBuf,
        path: PathBuf,
    }
    impl Fixture {
        fn new(data: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "agentvault-pi-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
            let path = root.join("sessions/project/sample.jsonl");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, data).unwrap();
            Self { root, path }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn pi_active_branch_preserves_physical_lines_and_readonly_metadata() {
        let f = Fixture::new(SAMPLE);
        assert!(validate_preview(f.path.to_str().unwrap()).is_err());
        let summary = parse_session(&f.root, &f.path, None).unwrap().unwrap();
        assert_eq!(summary.provider, "pi");
        assert_eq!(summary.title, "Pi fixture");
        assert!(summary.resume_command.is_empty());
        let e = events(f.path.to_str().unwrap(), None).unwrap();
        assert_eq!(e.iter().map(|e| e.index).collect::<Vec<_>>(), vec![2, 5, 6]);
        assert_eq!(crate::rollout::preview_event_text(&e[1]), "current answer");
        assert!(!e
            .iter()
            .any(|e| crate::rollout::preview_event_text(e).contains("obsolete")));
        assert_eq!(
            preview_range(f.path.to_str().unwrap(), 1, 1).unwrap()[0].index,
            5
        );
        assert_eq!(
            preview_meta(f.path.to_str().unwrap())
                .unwrap()
                .id
                .as_deref(),
            Some("sample")
        );
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), SAMPLE);
    }
    #[test]
    fn pi_rejects_cancel_corruption_partial_tail_and_foreign_paths() {
        let f = Fixture::new(SAMPLE);
        assert!(matches!(
            parse_session(&f.root, &f.path, Some(&AtomicBool::new(true))),
            Err(AppError::Cancelled)
        ));
        assert!(!is_main_transcript(&f.root, &f.root.join("other.jsonl")));
        for invalid in [
            format!("{SAMPLE}{{"),
            format!("{SAMPLE}broken\n"),
            SAMPLE.replace("\"parentId\":\"root\"", "\"parentId\":\"missing\""),
        ] {
            std::fs::write(&f.path, invalid).unwrap();
            assert!(parse_session(&f.root, &f.path, None).is_err());
        }
    }
    #[test]
    fn pi_projection_rejects_changed_source_instead_of_mislocating_hits() {
        let f = Fixture::new(SAMPLE);
        let parsed = read(&f.root, &f.path, None).unwrap();
        std::fs::write(&f.path, SAMPLE.replace("current answer", "changed answer")).unwrap();
        assert!(project(&parsed, &f.path, None).is_err());
    }
}
