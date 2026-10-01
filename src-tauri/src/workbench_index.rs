//! Rebuildable workbench search cache, never a writer of native sessions.

use std::{
    fs::{self, File, Metadata},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use registry::{
    CanonicalEvent, FileProjection, MachineRecord, NativeSessionRecord, ProjectionMode, Registry,
    SourceCursor, SourceInstanceRecord,
};

use serde::Serialize;

use sha2::{Digest, Sha256};

use crate::{
    content_search,
    error::{AppError, AppResult},
    models::{ContentSearchMatch, ProviderDirs, SessionSummary},
    rollout,
};

const PARSER: &str = "workbench-conversation-v1";
const PROJECTED_SOURCE_MAX_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const PROVIDERS: [&str; 8] = [
    "codex",
    "claude",
    "qoder",
    "workbuddy",
    "grok",
    "pi",
    "qwen",
    "copilot",
];
pub(crate) fn supports(provider: &str) -> bool {
    PROVIDERS.contains(&provider)
}

static INDEX_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn configure_index_path(path: PathBuf) -> AppResult<()> {
    if !path.is_absolute() {
        return Err(AppError::Path("搜索索引路径必须为绝对路径".into()));
    }

    if let Some(existing) = INDEX_PATH.get() {
        if existing == &path {
            return Ok(());
        }
        return Err(AppError::Other("搜索索引路径已经初始化".into()));
    }

    INDEX_PATH
        .set(path)
        .map_err(|_| AppError::Other("搜索索引路径已经初始化".into()))
}

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Other(format!("搜索索引: {e}"))
}

pub(crate) fn open() -> AppResult<Registry> {
    #[cfg(test)]
    if INDEX_PATH.get().is_none() {
        return Registry::open_in_memory().map_err(err);
    }
    let path = INDEX_PATH
        .get()
        .ok_or_else(|| err("宿主尚未配置索引目录"))?;

    fs::create_dir_all(path.parent().ok_or_else(|| err("索引目录无效"))?)?;

    Registry::open(path).map_err(err)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn source_id(provider: &str, root: &Path) -> AppResult<String> {
    let root = fs::canonicalize(root)?;

    Ok(format!(
        "workbench:{}:{}",
        provider,
        hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()))
    ))
}

pub fn root(dirs: &ProviderDirs, provider: &str) -> PathBuf {
    PathBuf::from(match provider {
        "codex" => dirs.codex_dir.as_str(),
        "claude" => dirs.claude_dir.as_deref().unwrap_or(""),
        "qoder" => dirs.qoder_dir.as_deref().unwrap_or(""),
        "workbuddy" => dirs.workbuddy_dir.as_deref().unwrap_or(""),
        "grok" => dirs.grok_dir.as_deref().unwrap_or(""),
        "pi" => dirs.pi_dir.as_deref().unwrap_or(""),
        "qwen" => dirs.qwen_dir.as_deref().unwrap_or(""),
        "copilot" => dirs.copilot_dir.as_deref().unwrap_or(""),
        _ => "",
    })
}

#[derive(Serialize)]

pub struct CachedSessions {
    pub sessions: Vec<SessionSummary>,
    pub index_updated_at_ms: Option<i64>,
}

pub fn cached_sessions(dirs: &ProviderDirs) -> AppResult<CachedSessions> {
    if INDEX_PATH.get().is_none_or(|p| !p.exists()) {
        return Ok(CachedSessions {
            sessions: vec![],
            index_updated_at_ms: None,
        });
    }

    cached_from(&open()?, dirs)
}

fn cached_from(db: &Registry, dirs: &ProviderDirs) -> AppResult<CachedSessions> {
    let mut out = CachedSessions {
        sessions: vec![],
        index_updated_at_ms: None,
    };

    for provider in PROVIDERS {
        let root = root(dirs, provider);

        if !root.is_absolute() || !root.is_dir() {
            continue;
        }

        for pk in db
            .session_ids_for_source(&source_id(provider, &root)?)
            .map_err(err)?
        {
            if let Some(record) = db.native_session(pk).map_err(err)? {
                let session: SessionSummary =
                    serde_json::from_value(record.metadata).map_err(err)?;

                if register_source(provider, &root, Path::new(&session.rollout_path)).is_err() {
                    continue;
                }
                if let Some(cursor) = db.source_cursor(pk, &session.rollout_path).map_err(err)? {
                    out.index_updated_at_ms = Some(
                        out.index_updated_at_ms
                            .unwrap_or(0)
                            .max(cursor.last_seen_at_ms),
                    );

                    out.sessions.push(session);
                }
            }
        }
    }

    Ok(out)
}

fn register_source(provider: &str, root: &Path, path: &Path) -> AppResult<()> {
    match provider {
        "qoder" => crate::qoder_sessions::register_source(root, path),
        "workbuddy" => crate::workbuddy_sessions::register_source(root, path),
        "grok" => crate::grok_sessions::register_source(root, path),
        "pi" => crate::pi_sessions::register_source(root, path),
        "qwen" => crate::qwen_sessions::register_source(root, path),
        "copilot" => crate::copilot_sessions::register_source(root, path),
        "codex" | "claude" => Ok(()),
        _ => Err(err("该来源不支持持久索引")),
    }
}
fn source_fingerprint(meta: &Metadata, session: &SessionSummary) -> AppResult<Fingerprint> {
    let mut fp = fingerprint(meta)?;
    if session.provider == "grok" {
        // Companion metadata controls visibility even when the transcript is unchanged.
        let summary = fingerprint(&fs::metadata(
            Path::new(&session.rollout_path).with_file_name("summary.json"),
        )?)?;
        fp.identity = format!(
            "{}:summary:{}:{}:{}",
            fp.identity, summary.identity, summary.size, summary.mtime
        );
    }
    Ok(fp)
}
fn push_conversation(
    events: &mut Vec<CanonicalEvent>,
    event: crate::models::PreviewEvent,
    offset: usize,
) {
    let mixed = rollout::preview_event_has_assistant_text_tool_use(&event);
    if !rollout::preview_event_is_conversation(&event) && !mixed {
        return;
    }
    let text = rollout::preview_event_text(&event);
    events.push(CanonicalEvent {
        event_id: event.index.to_string(),
        branch_id: None,
        native_event_id: None,
        parent_event_id: None,
        ordinal: offset as i64,
        timestamp_ms: None,
        kind: "conversation".into(),
        role: Some(if mixed {
            "assistant".into()
        } else {
            event.role
        }),
        plain_text: Some(text),
        tool_call_id: None,
        structured: serde_json::json!({"event_index": event.index, "timestamp": event.timestamp}),
        raw_byte_start: None,
        raw_byte_end: None,
        parse_quality: "complete".into(),
    });
}

#[derive(PartialEq, Eq)]

struct Fingerprint {
    size: u64,
    mtime: i64,
    identity: String,
}

fn fingerprint(meta: &Metadata) -> AppResult<Fingerprint> {
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(err)?
        .as_nanos()
        .min(i64::MAX as u128) as i64;

    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };

    #[cfg(windows)]
    let identity = {
        use std::os::windows::fs::MetadataExt;
        format!("{}", meta.creation_time())
    };

    #[cfg(not(any(unix, windows)))]
    let identity = format!("{:?}", meta.created()?);

    Ok(Fingerprint {
        size: meta.len(),
        mtime,
        identity,
    })
}

pub struct IndexOutcome {
    pub matches: Vec<ContentSearchMatch>,
    pub reused: bool,
    pub updated_at_ms: i64,
}

pub fn search(
    session: &SessionSummary,
    root: &Path,
    query: &str,
    cancel: &AtomicBool,
) -> AppResult<IndexOutcome> {
    search_with(&mut open()?, session, root, query, cancel)
}

pub(crate) fn search_with(
    db: &mut Registry,
    session: &SessionSummary,
    root: &Path,
    query: &str,
    cancel: &AtomicBool,
) -> AppResult<IndexOutcome> {
    refresh_and_search(db, session, root, Some(query), cancel)
}

// Once truncation is confirmed, keep refreshing files without materializing
// results that cannot be displayed. Cancellation and file checks still apply.
pub(crate) fn refresh_and_search(
    db: &mut Registry,
    session: &SessionSummary,
    root: &Path,
    query: Option<&str>,
    cancel: &AtomicBool,
) -> AppResult<IndexOutcome> {
    if !root.is_absolute() {
        return Err(err("索引来源目录必须是已配置的绝对路径"));
    }
    crate::path_safety::validate_descendant(
        root,
        Path::new(&session.rollout_path),
        crate::path_safety::EntryKind::File,
        false,
        "索引来源文件",
    )?;

    if cancel.load(Ordering::Acquire) {
        return Err(AppError::Cancelled);
    }

    let source = source_id(&session.provider, root)?;

    let existing = db
        .session_pk_for_source(&source, &session.rollout_path)
        .map_err(err)?;
    let file = File::open(&session.rollout_path)?;

    let before = source_fingerprint(&file.metadata()?, session)?;
    register_source(&session.provider, root, Path::new(&session.rollout_path))?;
    // Keep the adapters' 128 MiB bound before the hashing pass can allocate a line.
    let bounded_projection = matches!(session.provider.as_str(), "qwen" | "copilot");
    if bounded_projection && before.size > PROJECTED_SOURCE_MAX_BYTES {
        return Err(err("会话超过 128 MiB 安全读取上限"));
    }

    let cursor = existing
        .map(|pk| db.source_cursor(pk, &session.rollout_path))
        .transpose()
        .map_err(err)?
        .flatten();

    let reused = cursor.as_ref().is_some_and(|c| {
        c.size == before.size
            && c.mtime_ns == before.mtime
            && c.file_identity.as_deref() == Some(&before.identity)
            && c.parser_version == PARSER
    });

    let (pk, updated_at_ms) = if reused {
        (existing.unwrap(), cursor.unwrap().last_seen_at_ms)
    } else {
        // Bound the actual read as well: the native writer may append after metadata().
        let read_limit = if bounded_projection {
            PROJECTED_SOURCE_MAX_BYTES + 1
        } else {
            u64::MAX
        };
        let mut reader = BufReader::new(file.take(read_limit));
        let mut bytes_read = 0u64;

        let mut line = String::new();

        let mut events = Vec::new();

        let mut index = 0usize;

        let mut offset = 0usize;

        let mut hash = Sha256::new();

        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(AppError::Cancelled);
            }

            line.clear();

            let count = reader.read_line(&mut line)?;
            if count == 0 {
                break;
            }
            bytes_read += count as u64;
            if bounded_projection && bytes_read > PROJECTED_SOURCE_MAX_BYTES {
                return Err(err("会话超过 128 MiB 安全读取上限"));
            }

            hash.update(line.as_bytes());

            let current_index = index;
            index += 1;

            if line.trim().is_empty() {
                continue;
            }

            // These providers validate and project the complete native transcript below.
            // Qwen permits several complete JSON objects on one physical line.
            if matches!(session.provider.as_str(), "qwen" | "copilot") {
                continue;
            }
            let raw = serde_json::from_str(&line)
                .map_err(|e| err(format!("第 {index} 行 JSON 无效: {e}")))?;
            if let Some(event) =
                content_search::classify_event(&session.provider, current_index, raw)
            {
                push_conversation(&mut events, event, offset);
                offset += 1;
            }
        }
        // Branch/chunk providers must use the same projection as their previews.
        let projected = match session.provider.as_str() {
            "grok" => Some(crate::grok_sessions::events(
                &session.rollout_path,
                Some(cancel),
            )?),
            "pi" => Some(crate::pi_sessions::events(
                &session.rollout_path,
                Some(cancel),
            )?),
            "qwen" => Some(crate::qwen_sessions::events(
                &session.rollout_path,
                Some(cancel),
            )?),
            "copilot" => Some(crate::copilot_sessions::events(
                &session.rollout_path,
                Some(cancel),
            )?),
            _ => None,
        };
        if let Some(projected) = projected {
            for (offset, event) in projected.into_iter().enumerate() {
                if cancel.load(Ordering::Acquire) {
                    return Err(AppError::Cancelled);
                }
                push_conversation(&mut events, event, offset);
            }
        }

        if before != source_fingerprint(&reader.get_ref().get_ref().metadata()?, session)?
            || before != source_fingerprint(&fs::metadata(&session.rollout_path)?, session)?
        {
            return Err(err("文件读取期间发生变化；保留旧索引，本次不使用旧结果"));
        }

        if cancel.load(Ordering::Acquire) {
            return Err(AppError::Cancelled);
        }

        let at = now();

        db.upsert_machine(&MachineRecord {
            id: "workbench-local".into(),
            display_name: "Local workbench".into(),
            platform: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            observed_at_ms: at,
        })
        .map_err(err)?;

        db.upsert_source_instance(&SourceInstanceRecord {
            id: source.clone(),
            machine_id: "workbench-local".into(),
            provider_id: session.provider.clone(),
            config_root: fs::canonicalize(root)?.to_string_lossy().into_owned(),
            root_fingerprint: source.clone(),
            observed_at_ms: at,
        })
        .map_err(err)?;

        let pk = if let Some(pk) = existing {
            pk
        } else {
            db.upsert_native_session(&NativeSessionRecord {
                machine_id: "workbench-local".into(),
                source_instance_id: source,
                provider_id: session.provider.clone(),
                native_session_id: session.rollout_path.clone(),
                project_id: None,
                root_native_session_id: None,
                parent_native_session_id: None,
                title: Some(session.title.clone()),
                cwd_at_start: Some(session.cwd.clone()),
                model: session.model.clone(),
                created_at_ms: Some(session.created_at),
                updated_at_ms: Some(session.updated_at),
                source_format_version: None,
                parser_version: PARSER.into(),
                health_status: "healthy".into(),
                resumability: "unknown".into(),
                capabilities: Default::default(),
                metadata: serde_json::to_value(session)?,
            })
            .map_err(err)?
        };

        let content = events
            .iter()
            .filter_map(|e| e.plain_text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        db.commit_search_file_projection(
            &FileProjection {
                native_session_pk: pk,
                role: "conversation".into(),
                absolute_path: session.rollout_path.clone(),
                mode: ProjectionMode::Rebuild,
                verified_previous_hash: None,
                cursor: SourceCursor {
                    file_identity: Some(before.identity.clone()),
                    size: before.size,
                    mtime_ns: before.mtime,
                    parsed_offset: before.size,
                    last_complete_line_offset: before.size,
                    partial_tail: vec![],
                    parser_version: PARSER.into(),
                    last_hash: hex::encode(hash.finalize()),
                    last_seen_at_ms: at,
                },
                events,
            },
            Some(&content),
            Some(&serde_json::to_value(session)?),
        )
        .map_err(err)?;

        (pk, at)
    };

    let mut matches = Vec::new();

    if cancel.load(Ordering::Acquire) {
        return Err(AppError::Cancelled);
    }
    if let Some(query) = query {
        if db.body_matches(pk, query).map_err(err)? {
            for event in db.events_for_session(pk).map_err(err)? {
                if cancel.load(Ordering::Acquire) {
                    return Err(AppError::Cancelled);
                }

                let text = event.plain_text.unwrap_or_default();

                if content_search::find_query(&text, query).is_some() {
                    matches.push(ContentSearchMatch {
                        event_index: event.structured["event_index"].as_u64().unwrap_or(0) as usize,
                        event_offset: event.ordinal as usize,
                        timestamp: serde_json::from_value(event.structured["timestamp"].clone())?,
                        role: event.role.unwrap_or_default(),
                        snippet: content_search::make_snippet(&text, query),
                    });

                    if matches.len() == 3 {
                        break;
                    }
                }
            }
        }
    }

    if before != source_fingerprint(&fs::metadata(&session.rollout_path)?, session)? {
        return Err(err("查询期间文件发生变化，请重试"));
    }
    Ok(IndexOutcome {
        matches,
        reused,
        updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "av-index-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn session(path: &Path, provider: &str) -> SessionSummary {
        serde_json::from_value(serde_json::json!({"provider":provider,"id":"fixture","rollout_path":path.to_string_lossy(),"cwd":"/fixture","cwd_display":"fixture","title":"Fixture","first_user_message":"","model":null,"reasoning_effort":null,"source":null,"agent_nickname":null,"agent_role":null,"conversion_origin":null,"tokens_used":0,"created_at":0,"updated_at":0,"archived":false,"git_branch":null,"rollout_bytes":fs::metadata(path).unwrap().len(),"logs_count":0,"has_backup":false,"resume_command":""})).unwrap()
    }
    fn body(provider: &str, text: &str) -> String {
        let raw = if provider == "codex" {
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
        } else {
            serde_json::json!({"type":"user","message":{"role":"user","content":text}})
        };
        format!("{}\n", raw)
    }
    #[test]
    fn restart_incremental_literal_and_failed_tail_preserve_projection() {
        for provider in ["codex", "claude"] {
            let root = temp();
            let file = root.join("session.jsonl");
            let dbpath = root.join("registry.sqlite3");
            fs::write(&file, body(provider, "你好 foo::bar quote\"x A_B 100% 🚀")).unwrap();
            let session = session(&file, provider);
            let cancel = AtomicBool::new(false);
            {
                let mut db = Registry::open(&dbpath).unwrap();
                let r = search_with(&mut db, &session, &root, "你好", &cancel).unwrap();
                assert!(!r.reused);
                assert_eq!(r.matches.len(), 1);
                assert_eq!(r.matches[0].event_offset, 0);
            }
            let mut db = Registry::open(&dbpath).unwrap();
            for q in ["你好", "foo::bar", "quote\"x", "A_B", "100%", "🚀"] {
                let r = search_with(&mut db, &session, &root, q, &cancel).unwrap();
                assert!(r.reused);
                assert_eq!(r.matches.len(), 1, "{q}");
            }
            fs::write(
                &file,
                format!("{}{}", body(provider, "new content"), "{\"partial\":"),
            )
            .unwrap();
            assert!(search_with(&mut db, &session, &root, "new", &cancel).is_err());
            let pk = db
                .session_pk_for_source(&source_id(provider, &root).unwrap(), &session.rollout_path)
                .unwrap()
                .unwrap();
            assert!(db.body_matches(pk, "你好").unwrap());
            assert!(!db.body_matches(pk, "new").unwrap());
            fs::write(&file, body(provider, "replacement content")).unwrap();
            let r = search_with(&mut db, &session, &root, "replacement", &cancel).unwrap();
            assert!(!r.reused);
            assert_eq!(r.matches.len(), 1);
            assert!(!db.body_matches(pk, "你好").unwrap());
            fs::write(&file, body(provider, "refresh after result limit")).unwrap();
            let refreshed = refresh_and_search(&mut db, &session, &root, None, &cancel).unwrap();
            assert!(!refreshed.reused);
            assert!(refreshed.matches.is_empty());
            assert!(db.body_matches(pk, "result limit").unwrap());
            cancel.store(true, Ordering::Release);
            assert!(matches!(
                search_with(&mut db, &session, &root, "replacement", &cancel),
                Err(AppError::Cancelled)
            ));
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn root_scope_and_parser_version_are_isolated() {
        let root = temp();
        let first = root.join("one");
        let second = root.join("two");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let file = first.join("session.jsonl");
        fs::write(&file, body("codex", "needle")).unwrap();
        let session = session(&file, "codex");
        let cancel = AtomicBool::new(false);
        let mut db = Registry::open(root.join("registry.sqlite3")).unwrap();
        search_with(&mut db, &session, &first, "needle", &cancel).unwrap();
        assert!(search_with(&mut db, &session, &second, "needle", &cancel).is_err());
        let pk = db
            .session_pk_for_source(&source_id("codex", &first).unwrap(), &session.rollout_path)
            .unwrap()
            .unwrap();
        let mut cursor = db
            .source_cursor(pk, &session.rollout_path)
            .unwrap()
            .unwrap();
        cursor.parser_version = "old".into();
        db.commit_file_projection(&FileProjection {
            native_session_pk: pk,
            role: "conversation".into(),
            absolute_path: session.rollout_path.clone(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor,
            events: vec![],
        })
        .unwrap();
        assert!(
            !search_with(&mut db, &session, &first, "needle", &cancel)
                .unwrap()
                .reused
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    #[test]
    fn failed_fts_commit_rolls_back_cursor_events_and_metadata() {
        let dir =
            std::env::temp_dir().join(format!("av-index-atomic-{}-{}", std::process::id(), now()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("source.jsonl");
        let old =
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"old needle\"}}\n";
        fs::write(&file, old).unwrap();
        let session: SessionSummary = serde_json::from_value(serde_json::json!({"provider":"claude","id":"atomic","rollout_path":file,"cwd":"/fixture","cwd_display":"fixture","title":"Fixture","first_user_message":"","tokens_used":0,"created_at":0,"updated_at":0,"archived":false,"rollout_bytes":old.len(),"logs_count":0,"has_backup":false,"resume_command":""})).unwrap();
        let path = dir.join("registry.sqlite3");
        let mut db = Registry::open(&path).unwrap();
        let cancel = AtomicBool::new(false);
        search_with(&mut db, &session, &dir, "needle", &cancel).unwrap();
        let pk = db
            .session_pk_for_source(&source_id("claude", &dir).unwrap(), &session.rollout_path)
            .unwrap()
            .unwrap();
        let cursor = db.source_cursor(pk, &session.rollout_path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_test BEFORE UPDATE OF content ON session_search_projection BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        fs::write(&file, old.replace("old needle", "new replacement content")).unwrap();
        assert!(search_with(&mut db, &session, &dir, "replacement", &cancel).is_err());
        assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
        assert!(db.body_matches(pk, "needle").unwrap());
        assert!(!db.body_matches(pk, "replacement").unwrap());
        assert_eq!(
            db.events_for_session(pk).unwrap()[0].plain_text.as_deref(),
            Some("old needle")
        );
        conn.execute_batch("DROP TRIGGER fail_test;").unwrap();
        assert!(
            !search_with(&mut db, &session, &dir, "replacement", &cancel)
                .unwrap()
                .reused
        );
        drop(conn);
        drop(db);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod additional_provider_tests {
    use super::*;
    use serde_json::json;

    const PI: &str = concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"sample\",\"cwd\":\"/fixture\"}\n",
        "{\"type\":\"message\",\"id\":\"root\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"question\"}}\n",
        "{\"type\":\"message\",\"id\":\"old\",\"parentId\":\"root\",\"message\":{\"role\":\"assistant\",\"content\":\"obsolete answer\"}}\n",
        "{\"type\":\"message\",\"id\":\"new\",\"parentId\":\"root\",\"message\":{\"role\":\"assistant\",\"content\":\"current answer\"}}\n"
    );
    fn preview(provider: &str, path: &str, offset: usize) -> Vec<crate::models::PreviewEvent> {
        match provider {
            "qoder" => crate::qoder_sessions::preview_range(path, offset, 1),
            "workbuddy" => crate::workbuddy_sessions::preview_range(path, offset, 1),
            "grok" => crate::grok_sessions::preview_range(path, offset, 1),
            "pi" => crate::pi_sessions::preview_range(path, offset, 1),
            "qwen" => crate::qwen_sessions::preview_range(path, offset, 1),
            "copilot" => crate::copilot_sessions::preview_range(path, offset, 1),
            _ => unreachable!(),
        }
        .unwrap()
    }
    #[test]
    fn additional_providers_reuse_restart_refresh_and_preserve_preview_offsets() {
        for (provider, relative, body, query, excluded) in [
            (
                "qoder",
                "projects/demo/sample.jsonl",
                include_str!("../tests/fixtures/qoder/sample-qoder-session.jsonl"),
                "你好",
                "Caveat",
            ),
            (
                "workbuddy",
                "projects/demo/sample.jsonl",
                include_str!("../tests/fixtures/workbuddy/sample.jsonl"),
                "needle",
                "hidden",
            ),
            (
                "grok",
                "sessions/demo/s/updates.jsonl",
                include_str!("../tests/fixtures/grok/updates.jsonl"),
                "world",
                "private tool output",
            ),
            (
                "pi",
                "sessions/demo/sample.jsonl",
                PI,
                "current answer",
                "obsolete answer",
            ),
            (
                "qwen",
                "projects/demo/chats/019f0000-0000-7000-8000-000000000001.jsonl",
                include_str!("../tests/fixtures/qwen/019f0000-0000-7000-8000-000000000001.jsonl"),
                "active answer",
                "dead branch",
            ),
            (
                "copilot",
                "session-state/copilot_stage0_small/events.jsonl",
                include_str!("../tests/fixtures/copilot/copilot_stage0_small.jsonl"),
                "Listing the files.",
                "trimmed for fixture",
            ),
        ] {
            let root = std::env::temp_dir().join(format!(
                "av-index-four-{provider}-{}-{}",
                std::process::id(),
                now()
            ));
            let file = root.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, body).unwrap();
            if provider == "grok" {
                fs::write(
                    file.with_file_name("summary.json"),
                    include_str!("../tests/fixtures/grok/summary.json"),
                )
                .unwrap();
            }
            let session: SessionSummary = serde_json::from_value(json!({"provider":provider,"id":"fixture","rollout_path":file,"cwd":"/fixture","cwd_display":"fixture","title":"Fixture","first_user_message":"","tokens_used":0,"created_at":0,"updated_at":0,"archived":false,"rollout_bytes":body.len(),"logs_count":0,"has_backup":false,"resume_command":""})).unwrap();
            let cancel = AtomicBool::new(false);
            let dbpath = root.join("index.sqlite3");
            let mut db = Registry::open(&dbpath).unwrap();
            let cold = search_with(&mut db, &session, &root, query, &cancel).unwrap();
            assert!(!cold.reused, "{provider}");
            assert!(!cold.matches.is_empty(), "{provider}");
            for hit in &cold.matches {
                let projected = preview(provider, file.to_str().unwrap(), hit.event_offset);
                assert_eq!(projected[0].index, hit.event_index, "{provider}");
                assert!(rollout::preview_event_text(&projected[0]).contains(query));
            }
            assert!(search_with(&mut db, &session, &root, excluded, &cancel)
                .unwrap()
                .matches
                .is_empty());
            assert_eq!(fs::read_to_string(&file).unwrap(), body);
            drop(db);
            let mut db = Registry::open(&dbpath).unwrap();
            assert!(
                search_with(&mut db, &session, &root, query, &cancel)
                    .unwrap()
                    .reused
            );
            let mut dirs = ProviderDirs::default();
            match provider {
                "qoder" => dirs.qoder_dir = Some(root.to_string_lossy().into_owned()),
                "workbuddy" => dirs.workbuddy_dir = Some(root.to_string_lossy().into_owned()),
                "grok" => dirs.grok_dir = Some(root.to_string_lossy().into_owned()),
                "pi" => dirs.pi_dir = Some(root.to_string_lossy().into_owned()),
                "qwen" => dirs.qwen_dir = Some(root.to_string_lossy().into_owned()),
                "copilot" => dirs.copilot_dir = Some(root.to_string_lossy().into_owned()),
                _ => unreachable!(),
            }
            match provider {
                "qwen" => crate::qwen_sessions::forget_source_for_test(&root),
                "copilot" => crate::copilot_sessions::forget_source_for_test(&root),
                _ => {}
            }
            assert_eq!(cached_from(&db, &dirs).unwrap().sessions.len(), 1);
            assert!(!preview(provider, file.to_str().unwrap(), 0).is_empty());
            assert!(cached_from(&db, &ProviderDirs::default())
                .unwrap()
                .sessions
                .is_empty());
            let changed = body.replace(query, "replacement 中文 foo::bar");
            fs::write(&file, &changed).unwrap();
            let updated = search_with(&mut db, &session, &root, "replacement", &cancel).unwrap();
            assert!(!updated.reused);
            assert!(!updated.matches.is_empty());
            let pk = db
                .session_pk_for_source(&source_id(provider, &root).unwrap(), &session.rollout_path)
                .unwrap()
                .unwrap();
            let cursor = db.source_cursor(pk, &session.rollout_path).unwrap();
            fs::write(&file, format!("{changed}\n{{\"partial\":")).unwrap();
            assert!(search_with(&mut db, &session, &root, "replacement", &cancel).is_err());
            assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
            assert!(db.body_matches(pk, "replacement").unwrap());
            if matches!(provider, "qwen" | "copilot") {
                File::options()
                    .write(true)
                    .open(&file)
                    .unwrap()
                    .set_len(128 * 1024 * 1024 + 1)
                    .unwrap();
                let error = search_with(&mut db, &session, &root, "replacement", &cancel)
                    .err()
                    .expect("oversized source must be rejected");
                assert!(error.to_string().contains("128 MiB"));
                assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
            }
            fs::write(&file, &changed).unwrap();
            cancel.store(true, Ordering::Release);
            assert!(matches!(
                search_with(&mut db, &session, &root, "replacement", &cancel),
                Err(AppError::Cancelled)
            ));
            assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
            drop(db);
            fs::remove_dir_all(&root).unwrap();
        }
    }
}

#[cfg(test)]
mod grok_cache_visibility_tests {
    use super::*;
    #[test]
    fn companion_visibility_changes_cannot_reuse_old_hits() {
        let root = std::env::temp_dir().join(format!(
            "av-grok-visibility-{}-{}",
            std::process::id(),
            now()
        ));
        let file = root.join("sessions/demo/s/updates.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, include_str!("../tests/fixtures/grok/updates.jsonl")).unwrap();
        let summary = file.with_file_name("summary.json");
        let original = include_str!("../tests/fixtures/grok/summary.json");
        fs::write(&summary, original).unwrap();
        let session = crate::grok_sessions::parse_session(&root, &file, None)
            .unwrap()
            .unwrap();
        let mut db = Registry::open(root.join("index.sqlite3")).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            !search_with(&mut db, &session, &root, "world", &cancel)
                .unwrap()
                .reused
        );
        let mut metadata: serde_json::Value = serde_json::from_str(original).unwrap();
        metadata["hidden"] = true.into();
        fs::write(&summary, metadata.to_string()).unwrap();
        assert!(search_with(&mut db, &session, &root, "world", &cancel).is_err());
        let dirs = ProviderDirs {
            grok_dir: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert!(cached_from(&db, &dirs).unwrap().sessions.is_empty());
        fs::write(&summary, original).unwrap();
        assert!(
            !search_with(&mut db, &session, &root, "world", &cancel)
                .unwrap()
                .reused
        );
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", serde_json::json!({"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"rewind_marker","target_prompt_index":0}}})).unwrap();
        drop(f);
        assert!(search_with(&mut db, &session, &root, "world", &cancel)
            .unwrap()
            .matches
            .is_empty());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod qwen_copilot_projection_tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn qwen_concatenated_fragments_preserve_active_branch_and_reject_invalid_refresh() {
        let temp = crate::readonly_source::test_support::tempdir().unwrap();
        let root = temp.path();
        let file = root.join("projects/demo/chats/019f0000-0000-7000-8000-000000000001.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let body =
            include_str!("../tests/fixtures/qwen/019f0000-0000-7000-8000-000000000001.jsonl");
        let compact = format!("{}\n", body.lines().collect::<String>());
        fs::write(&file, &compact).unwrap();
        let session = crate::qwen_sessions::parse_session(root, &file, None)
            .unwrap()
            .unwrap();
        let mut db = Registry::open_in_memory().unwrap();
        let cancel = AtomicBool::new(false);
        let result = search_with(&mut db, &session, root, "fragment tail", &cancel).unwrap();
        assert_eq!(result.matches.len(), 1);
        let hit = &result.matches[0];
        let preview =
            crate::qwen_sessions::preview_range(file.to_str().unwrap(), hit.event_offset, 1)
                .unwrap();
        assert_eq!(preview[0].index, hit.event_index);
        assert!(rollout::preview_event_text(&preview[0]).contains("active answer"));
        assert!(rollout::preview_event_text(&preview[0]).contains("fragment tail"));
        for absent in [
            "dead branch",
            "hidden hook",
            "secret-tool",
            "Qwen synthetic branch",
        ] {
            assert!(search_with(&mut db, &session, root, absent, &cancel)
                .unwrap()
                .matches
                .is_empty());
        }
        let pk = db
            .session_pk_for_source(&source_id("qwen", root).unwrap(), &session.rollout_path)
            .unwrap()
            .unwrap();
        let cursor = db.source_cursor(pk, &session.rollout_path).unwrap();
        for broken in [
            format!("{}junk\n", compact.trim_end()),
            compact.replace("\"parentUuid\":\"tool\"", "\"parentUuid\":\"missing\""),
            compact.trim_end().to_owned(),
        ] {
            fs::write(&file, broken).unwrap();
            assert!(search_with(&mut db, &session, root, "active answer", &cancel).is_err());
            assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
            assert!(db.body_matches(pk, "active answer").unwrap());
        }
        fs::write(&file, &compact).unwrap();
        let foreign = root.join("other-root");
        fs::create_dir_all(&foreign).unwrap();
        assert!(search_with(&mut db, &session, &foreign, "active answer", &cancel).is_err());
        let dirs = ProviderDirs {
            qwen_dir: Some(foreign.to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert!(cached_from(&db, &dirs).unwrap().sessions.is_empty());
        assert_eq!(fs::read_to_string(file).unwrap(), compact);
    }

    #[test]
    fn copilot_index_excludes_injected_messages_in_both_native_layouts() {
        for relative in [
            "session-state/copilot_stage0_small/events.jsonl",
            "session-state/copilot_stage0_small.jsonl",
        ] {
            let temp = crate::readonly_source::test_support::tempdir().unwrap();
            let root = temp.path();
            let file = root.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut body =
                include_str!("../tests/fixtures/copilot/copilot_stage0_small.jsonl").to_owned();
            for (index, extra) in [
                json!({"agentId":"background"}),
                json!({"ephemeral":true}),
                json!({"data":{"isAutopilotContinuation":true}}),
                json!({"data":{"source":"skill-injection"}}),
                json!({"data":{"source":"agent-injection"}}),
            ]
            .into_iter()
            .enumerate()
            {
                let mut row = json!({"id":format!("injected-{index}"),"type":"user.message","timestamp":"2026-09-24T00:00:00Z","parentId":null,"data":{"content":"hidden injected content"}});
                for (key, value) in extra.as_object().unwrap() {
                    if key == "data" {
                        for (k, v) in value.as_object().unwrap() {
                            row["data"][k] = v.clone();
                        }
                    } else {
                        row[key] = value.clone();
                    }
                }
                body.push_str(&format!("{row}\n"));
            }
            fs::write(&file, &body).unwrap();
            let session = crate::copilot_sessions::parse_session(root, &file, None)
                .unwrap()
                .unwrap();
            let mut db = Registry::open_in_memory().unwrap();
            let cancel = AtomicBool::new(false);
            let result =
                search_with(&mut db, &session, root, "Listing the files.", &cancel).unwrap();
            assert_eq!(result.matches.len(), 1);
            let hit = &result.matches[0];
            let preview =
                crate::copilot_sessions::preview_range(file.to_str().unwrap(), hit.event_offset, 1)
                    .unwrap();
            assert_eq!(hit.event_offset, 8);
            assert_eq!(preview[0].index, hit.event_index);
            for absent in ["hidden injected", "a.txt", "trimmed for fixture"] {
                assert!(search_with(&mut db, &session, root, absent, &cancel)
                    .unwrap()
                    .matches
                    .is_empty());
            }
            let pk = db
                .session_pk_for_source(&source_id("copilot", root).unwrap(), &session.rollout_path)
                .unwrap()
                .unwrap();
            let cursor = db.source_cursor(pk, &session.rollout_path).unwrap();
            let duplicate: Value = serde_json::from_str(body.lines().nth(6).unwrap()).unwrap();
            fs::write(&file, format!("{body}{duplicate}\n")).unwrap();
            assert!(search_with(&mut db, &session, root, "Listing", &cancel).is_err());
            assert_eq!(db.source_cursor(pk, &session.rollout_path).unwrap(), cursor);
            assert!(db.body_matches(pk, "Listing").unwrap());
            fs::write(&file, &body).unwrap();
            let foreign = root.join("other-root");
            fs::create_dir_all(&foreign).unwrap();
            assert!(search_with(&mut db, &session, &foreign, "Listing", &cancel).is_err());
            let dirs = ProviderDirs {
                copilot_dir: Some(foreign.to_string_lossy().into_owned()),
                ..Default::default()
            };
            assert!(cached_from(&db, &dirs).unwrap().sessions.is_empty());
            assert_eq!(fs::read_to_string(file).unwrap(), body);
        }
    }
}
