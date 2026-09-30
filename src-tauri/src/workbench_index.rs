//! Rebuildable Codex/Claude search cache, never a writer of native sessions.

use std::{
    fs::{self, File, Metadata},
    io::{BufRead, BufReader},
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
    if provider == "codex" {
        PathBuf::from(&dirs.codex_dir)
    } else {
        PathBuf::from(dirs.claude_dir.as_deref().unwrap_or(""))
    }
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

    for provider in ["codex", "claude"] {
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

    let before = fingerprint(&file.metadata()?)?;

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
        let mut reader = BufReader::new(file);

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

            if reader.read_line(&mut line)? == 0 {
                break;
            }

            hash.update(line.as_bytes());

            let current_index = index;
            index += 1;

            if line.trim().is_empty() {
                continue;
            }

            let current_offset = offset;
            offset += 1;

            let raw = serde_json::from_str(&line)
                .map_err(|e| err(format!("第 {index} 行 JSON 无效: {e}")))?;

            if let Some(event) =
                content_search::classify_event(&session.provider, current_index, raw)
            {
                let mixed = rollout::preview_event_has_assistant_text_tool_use(&event);

                if !rollout::preview_event_is_conversation(&event) && !mixed {
                    continue;
                }

                let text = rollout::preview_event_text(&event);

                events.push(CanonicalEvent { event_id: current_index.to_string(), branch_id: None, native_event_id: None, parent_event_id: None, ordinal: current_offset as i64, timestamp_ms: None, kind: "conversation".into(), role: Some(if mixed { "assistant".into() } else { event.role }), plain_text: Some(text), tool_call_id: None, structured: serde_json::json!({"event_index":current_index,"timestamp":event.timestamp}), raw_byte_start: None, raw_byte_end: None, parse_quality: "complete".into() });
            }
        }

        if before != fingerprint(&reader.get_ref().metadata()?)?
            || before != fingerprint(&fs::metadata(&session.rollout_path)?)?
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

    if before != fingerprint(&fs::metadata(&session.rollout_path)?)? {
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
