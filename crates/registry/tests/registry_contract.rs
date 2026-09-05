use provider_sdk::ProviderCapabilities;
use registry::{
    CanonicalEvent, FileProjection, FullScanReason, MachineRecord, NativeSessionRecord,
    ProjectRecord, ProjectionMode, Registry, RegistryError, SourceCursor, SourceInstanceRecord,
    SourceObservation, SourceScanDecision, LATEST_SCHEMA_VERSION,
};
use rusqlite::Connection;
use serde_json::json;

fn seed_registry() -> (Registry, i64) {
    let mut registry = Registry::open_in_memory().expect("open registry");
    registry
        .upsert_machine(&MachineRecord {
            id: "machine-a".into(),
            display_name: "Laptop".into(),
            platform: "macos".into(),
            arch: "aarch64".into(),
            observed_at_ms: 10,
        })
        .expect("upsert machine");
    registry
        .upsert_source_instance(&SourceInstanceRecord {
            id: "source-a".into(),
            machine_id: "machine-a".into(),
            provider_id: "pi".into(),
            config_root: "/home/alice/.pi/agent".into(),
            root_fingerprint: "root-a".into(),
            observed_at_ms: 11,
        })
        .expect("upsert source");
    let session_pk = registry
        .upsert_native_session(&NativeSessionRecord {
            machine_id: "machine-a".into(),
            source_instance_id: "source-a".into(),
            provider_id: "pi".into(),
            native_session_id: "native-1".into(),
            project_id: None,
            root_native_session_id: None,
            parent_native_session_id: None,
            title: Some("First title".into()),
            cwd_at_start: Some("/workspace".into()),
            model: None,
            created_at_ms: Some(1),
            updated_at_ms: Some(2),
            source_format_version: Some("3".into()),
            parser_version: "pi-parser-v1".into(),
            health_status: "healthy".into(),
            resumability: "native".into(),
            capabilities: ProviderCapabilities::PARSE | ProviderCapabilities::BRANCH_GRAPH,
            metadata: json!({"branch_count": 2}),
        })
        .expect("upsert session");
    (registry, session_pk)
}

fn cursor(size: u64, mtime_ns: i64) -> SourceCursor {
    SourceCursor {
        file_identity: Some("file-identity-1".into()),
        size,
        mtime_ns,
        parsed_offset: size,
        last_complete_line_offset: size,
        partial_tail: Vec::new(),
        parser_version: "pi-parser-v1".into(),
        last_hash: format!("hash-{size}"),
        last_seen_at_ms: 50,
    }
}

fn event(id: &str, ordinal: i64, kind: &str) -> CanonicalEvent {
    CanonicalEvent {
        event_id: id.into(),
        branch_id: Some("main".into()),
        native_event_id: Some(format!("native-{id}")),
        parent_event_id: None,
        ordinal,
        timestamp_ms: Some(100 + ordinal),
        kind: kind.into(),
        role: None,
        plain_text: None,
        tool_call_id: (kind == "tool_result").then(|| "call-1".into()),
        structured: json!({"type": kind, "extension": {"keep": true}}),
        raw_byte_start: Some(0),
        raw_byte_end: Some(10),
        parse_quality: "lossless".into(),
    }
}

#[test]
fn initializes_versioned_core_schema_with_foreign_keys() {
    let registry = Registry::open_in_memory().expect("open registry");

    assert_eq!(
        registry.schema_version().expect("schema version"),
        LATEST_SCHEMA_VERSION
    );
    assert!(registry.foreign_keys_enabled().expect("foreign keys"));
    assert_eq!(
        registry.table_names().expect("table names"),
        vec![
            "machines",
            "native_sessions",
            "project_locations",
            "projects",
            "schema_migrations",
            "session_events",
            "session_events_fts",
            "session_events_trigram",
            "session_search_projection",
            "source_files",
            "source_instances",
        ]
    );
}

#[test]
fn refuses_unversioned_or_newer_databases_without_claiming_their_schema() {
    let unrelated = Connection::open_in_memory().expect("open unrelated database");
    unrelated
        .execute("CREATE TABLE keep_me (value TEXT NOT NULL)", [])
        .expect("create unrelated table");
    unrelated
        .execute("INSERT INTO keep_me(value) VALUES ('untouched')", [])
        .expect("seed unrelated table");

    let error = Registry::from_connection(unrelated)
        .err()
        .expect("unversioned database must be rejected");
    assert!(matches!(error, RegistryError::ConflictingSchema));

    let newer = Connection::open_in_memory().expect("open newer database");
    newer
        .pragma_update(None, "user_version", LATEST_SCHEMA_VERSION + 1)
        .expect("set newer schema version");
    let error = Registry::from_connection(newer)
        .err()
        .expect("newer database must be rejected");
    assert!(matches!(
        error,
        RegistryError::SchemaTooNew {
            found,
            supported: LATEST_SCHEMA_VERSION,
        } if found == LATEST_SCHEMA_VERSION + 1
    ));
}

#[test]
fn native_identity_is_scoped_by_machine_and_source_and_upserts_in_place() {
    let (mut registry, first_pk) = seed_registry();
    let mut updated = registry
        .native_session(first_pk)
        .expect("load session")
        .expect("session exists");
    updated.title = Some("Updated title".into());
    updated.capabilities = ProviderCapabilities::from_bits_retain(1 << 63);

    let same_pk = registry
        .upsert_native_session(&updated)
        .expect("update same identity");
    assert_eq!(same_pk, first_pk);
    let stored = registry
        .native_session(first_pk)
        .expect("load updated session")
        .expect("updated session exists");
    assert_eq!(stored.title.as_deref(), Some("Updated title"));
    assert_eq!(stored.capabilities.bits(), 1 << 63);

    registry
        .upsert_machine(&MachineRecord {
            id: "machine-b".into(),
            display_name: "Desktop".into(),
            platform: "linux".into(),
            arch: "x86_64".into(),
            observed_at_ms: 20,
        })
        .expect("upsert second machine");
    registry
        .upsert_source_instance(&SourceInstanceRecord {
            id: "source-b".into(),
            machine_id: "machine-b".into(),
            provider_id: "pi".into(),
            config_root: "/home/bob/.pi/agent".into(),
            root_fingerprint: "root-b".into(),
            observed_at_ms: 21,
        })
        .expect("upsert second source");
    let mut second = updated;
    second.machine_id = "machine-b".into();
    second.source_instance_id = "source-b".into();
    let second_pk = registry
        .upsert_native_session(&second)
        .expect("insert same native id from another machine");

    assert_ne!(second_pk, first_pk);
    assert_eq!(registry.native_session_count().expect("session count"), 2);
}

#[test]
fn native_session_can_reference_a_registered_canonical_project() {
    let (mut registry, session_pk) = seed_registry();
    registry
        .upsert_project(&ProjectRecord {
            id: "project-1".into(),
            display_name: "AgentVault".into(),
            normalized_remotes: vec!["https://example.com/agentvault.git".into()],
            root_commit: Some("abc123".into()),
            created_at_ms: 1,
            updated_at_ms: 2,
        })
        .expect("upsert project");
    let mut session = registry
        .native_session(session_pk)
        .expect("load session")
        .expect("session exists");
    session.project_id = Some("project-1".into());

    assert_eq!(
        registry
            .upsert_native_session(&session)
            .expect("link canonical project"),
        session_pk
    );
    assert_eq!(
        registry
            .native_session(session_pk)
            .expect("reload session")
            .expect("session exists")
            .project_id
            .as_deref(),
        Some("project-1")
    );
}

#[test]
fn source_cursor_selects_unchanged_resume_and_conservative_rebuild_paths() {
    let (mut registry, session_pk) = seed_registry();
    let path = "/home/alice/.pi/agent/sessions/live.jsonl";

    assert_eq!(
        registry
            .source_scan_decision(
                session_pk,
                path,
                &SourceObservation {
                    file_identity: Some("file-identity-1".into()),
                    size: 10,
                    mtime_ns: 100,
                    verified_last_hash: Some("hash-10".into()),
                },
                "pi-parser-v1",
            )
            .expect("initial decision"),
        SourceScanDecision::FullScan(FullScanReason::MissingCursor)
    );

    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: cursor(10, 100),
            events: vec![event("event-1", 0, "message")],
        })
        .expect("commit initial projection");

    let unchanged = SourceObservation {
        file_identity: Some("file-identity-1".into()),
        size: 10,
        mtime_ns: 100,
        verified_last_hash: Some("hash-10".into()),
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &unchanged, "pi-parser-v1")
            .expect("unchanged decision"),
        SourceScanDecision::Unchanged
    );

    let grown = SourceObservation {
        size: 25,
        mtime_ns: 110,
        verified_last_hash: Some("hash-10".into()),
        ..unchanged.clone()
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &grown, "pi-parser-v1")
            .expect("append decision"),
        SourceScanDecision::Resume {
            parsed_offset: 10,
            partial_tail: Vec::new(),
        }
    );

    let truncated = SourceObservation {
        size: 5,
        mtime_ns: 120,
        verified_last_hash: Some("hash-5".into()),
        ..unchanged.clone()
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &truncated, "pi-parser-v1")
            .expect("truncation decision"),
        SourceScanDecision::FullScan(FullScanReason::Truncated)
    );

    let replaced = SourceObservation {
        file_identity: Some("file-identity-2".into()),
        ..grown.clone()
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &replaced, "pi-parser-v1")
            .expect("replacement decision"),
        SourceScanDecision::FullScan(FullScanReason::FileReplaced)
    );
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &unchanged, "pi-parser-v2")
            .expect("parser decision"),
        SourceScanDecision::FullScan(FullScanReason::ParserVersionChanged)
    );

    let changed_prefix = SourceObservation {
        size: 25,
        mtime_ns: 110,
        verified_last_hash: Some("rewritten-prefix".into()),
        ..unchanged.clone()
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &changed_prefix, "pi-parser-v1")
            .expect("prefix verification decision"),
        SourceScanDecision::FullScan(FullScanReason::ContentChanged)
    );

    let unverified_prefix = SourceObservation {
        size: 25,
        mtime_ns: 110,
        verified_last_hash: None,
        ..unchanged
    };
    assert_eq!(
        registry
            .source_scan_decision(session_pk, path, &unverified_prefix, "pi-parser-v1")
            .expect("missing prefix verification decision"),
        SourceScanDecision::FullScan(FullScanReason::HashUnverified)
    );
}

#[test]
fn cursor_without_file_identity_never_resumes_changed_content() {
    let (mut registry, session_pk) = seed_registry();
    let path = "/home/alice/.pi/agent/sessions/no-identity.jsonl";
    let mut initial = cursor(10, 100);
    initial.file_identity = None;
    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: initial,
            events: vec![],
        })
        .expect("commit cursor without identity");

    let decision = registry
        .source_scan_decision(
            session_pk,
            path,
            &SourceObservation {
                file_identity: None,
                size: 20,
                mtime_ns: 110,
                verified_last_hash: Some("hash-10".into()),
            },
            "pi-parser-v1",
        )
        .expect("decision without identity");

    assert_eq!(
        decision,
        SourceScanDecision::FullScan(FullScanReason::IdentityUnavailable)
    );
}

#[test]
fn resume_returns_the_exact_unterminated_tail_and_next_unread_offset() {
    let (mut registry, session_pk) = seed_registry();
    let path = "/home/alice/.pi/agent/sessions/partial.jsonl";
    let partial_cursor = SourceCursor {
        file_identity: Some("file-identity-1".into()),
        size: 12,
        mtime_ns: 100,
        parsed_offset: 12,
        last_complete_line_offset: 10,
        partial_tail: b"{\"".to_vec(),
        parser_version: "pi-parser-v1".into(),
        last_hash: "hash-12".into(),
        last_seen_at_ms: 50,
    };
    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: partial_cursor,
            events: vec![],
        })
        .expect("commit partial tail");

    assert_eq!(
        registry
            .source_scan_decision(
                session_pk,
                path,
                &SourceObservation {
                    file_identity: Some("file-identity-1".into()),
                    size: 20,
                    mtime_ns: 110,
                    verified_last_hash: Some("hash-12".into()),
                },
                "pi-parser-v1",
            )
            .expect("resume decision"),
        SourceScanDecision::Resume {
            parsed_offset: 12,
            partial_tail: b"{\"".to_vec(),
        }
    );
}

#[test]
fn append_advances_events_and_cursor_but_rejects_a_replaced_file() {
    let (mut registry, session_pk) = seed_registry();
    let path = "/home/alice/.pi/agent/sessions/append.jsonl";
    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: cursor(10, 100),
            events: vec![event("event-1", 0, "message")],
        })
        .expect("commit initial projection");

    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Append,
            verified_previous_hash: Some("hash-10".into()),
            cursor: cursor(20, 110),
            events: vec![event("event-2", 1, "tool_result")],
        })
        .expect("append projection");
    let appended = registry
        .events_for_session(session_pk)
        .expect("events after append");
    assert_eq!(appended.len(), 2);
    assert_eq!(appended[1].tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        registry
            .source_cursor(session_pk, path)
            .expect("cursor after append")
            .expect("cursor exists")
            .size,
        20
    );

    let mut replaced_cursor = cursor(30, 120);
    replaced_cursor.file_identity = Some("replacement".into());
    let error = registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Append,
            verified_previous_hash: Some("hash-20".into()),
            cursor: replaced_cursor,
            events: vec![event("should-not-commit", 2, "message")],
        })
        .expect_err("replacement requires rebuild");

    assert!(error.to_string().contains("FileReplaced"));
    assert_eq!(
        registry
            .events_for_session(session_pk)
            .expect("events after rejection")
            .len(),
        2
    );
    assert_eq!(
        registry
            .source_cursor(session_pk, path)
            .expect("cursor after rejection")
            .expect("cursor remains")
            .size,
        20
    );
}

#[test]
fn projection_commit_is_atomic_and_preserves_unknown_events() {
    let (mut registry, session_pk) = seed_registry();
    let path = "/home/alice/.pi/agent/sessions/live.jsonl";
    let unknown = event("unknown-1", 0, "vendor.extension");
    registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: cursor(10, 100),
            events: vec![unknown.clone()],
        })
        .expect("commit projection");

    assert_eq!(
        registry.events_for_session(session_pk).expect("events"),
        vec![unknown]
    );
    let before_cursor = registry
        .source_cursor(session_pk, path)
        .expect("load cursor")
        .expect("cursor exists");

    let mut invalid_cursor = cursor(20, 200);
    invalid_cursor.parsed_offset = 21;
    let error = registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: path.into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: invalid_cursor,
            events: vec![event("should-not-commit", 1, "message")],
        })
        .expect_err("invalid cursor must reject the whole projection");

    assert!(error.to_string().contains("parsed_offset"));
    assert_eq!(
        registry
            .events_for_session(session_pk)
            .expect("events after rejection"),
        vec![event("unknown-1", 0, "vendor.extension")]
    );
    assert_eq!(
        registry
            .source_cursor(session_pk, path)
            .expect("cursor after rejection")
            .expect("cursor remains"),
        before_cursor
    );

    let mut unhashed_cursor = cursor(10, 100);
    unhashed_cursor.last_hash.clear();
    let error = registry
        .commit_file_projection(&FileProjection {
            native_session_pk: session_pk,
            role: "transcript".into(),
            absolute_path: "/home/alice/.pi/agent/sessions/unhashed.jsonl".into(),
            mode: ProjectionMode::Rebuild,
            verified_previous_hash: None,
            cursor: unhashed_cursor,
            events: vec![],
        })
        .expect_err("a stored cursor must include its verification hash");
    assert!(error.to_string().contains("last_hash"));
}
