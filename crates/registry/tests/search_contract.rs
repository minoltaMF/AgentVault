use provider_sdk::ProviderCapabilities;
use registry::{
    MachineRecord, NativeSessionRecord, ProjectRecord, Registry, RegistryError, SearchDocument,
    SourceInstanceRecord, LATEST_SCHEMA_VERSION,
};
use rusqlite::Connection;
use serde_json::json;

fn seed_registry() -> Registry {
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
    registry
}

fn native_session(id: &str, title: &str, project_id: Option<&str>) -> NativeSessionRecord {
    NativeSessionRecord {
        machine_id: "machine-a".into(),
        source_instance_id: "source-a".into(),
        provider_id: "pi".into(),
        native_session_id: id.into(),
        project_id: project_id.map(str::to_owned),
        root_native_session_id: None,
        parent_native_session_id: None,
        title: Some(title.into()),
        cwd_at_start: Some("/workspace".into()),
        model: None,
        created_at_ms: Some(1),
        updated_at_ms: Some(2),
        source_format_version: Some("3".into()),
        parser_version: "pi-parser-v1".into(),
        health_status: "healthy".into(),
        resumability: "native".into(),
        capabilities: ProviderCapabilities::PARSE,
        metadata: json!({}),
    }
}

fn replace_document(
    registry: &mut Registry,
    native_session_pk: i64,
    content: &str,
    tool_names: &[&str],
    paths: &[&str],
) {
    registry
        .replace_search_document(&SearchDocument {
            native_session_pk,
            content: content.into(),
            tool_names: tool_names.iter().map(|value| (*value).into()).collect(),
            paths: paths.iter().map(|value| (*value).into()).collect(),
        })
        .expect("replace search document");
}

#[test]
fn initializes_versioned_unicode_and_trigram_indexes() {
    let registry = Registry::open_in_memory().expect("open registry");

    assert_eq!(LATEST_SCHEMA_VERSION, 2);
    assert_eq!(registry.schema_version().expect("schema version"), 2);
    let tables = registry.table_names().expect("table names");
    assert!(tables.contains(&"session_search_projection".to_string()));
    assert!(tables.contains(&"session_events_fts".to_string()));
    assert!(tables.contains(&"session_events_trigram".to_string()));
    assert!(!tables
        .iter()
        .any(|name| name.starts_with("session_events_fts_")
            || name.starts_with("session_events_trigram_")));
}

#[test]
fn searches_unicode_terms_and_trigram_substrings_across_projection_fields() {
    let mut registry = seed_registry();
    registry
        .upsert_project(&ProjectRecord {
            id: "project-1".into(),
            display_name: "Swordfish Runtime".into(),
            normalized_remotes: vec![],
            root_commit: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        })
        .expect("upsert project");
    let session_pk = registry
        .upsert_native_session(&native_session(
            "native-1",
            "Runtime Session Binding",
            Some("project-1"),
        ))
        .expect("upsert session");
    replace_document(
        &mut registry,
        session_pk,
        "AgentVault 支持中文检索能力 and stable reconnect",
        &["WebSearch"],
        &["/workspace/src/parse_runtime.rs"],
    );

    for query in [
        "Runtime",
        "Swordfish",
        "WebSearch",
        "文检索",
        "ble reco",
        "src/parse",
    ] {
        let hits = registry.search_sessions(query, 10).expect("search");
        assert_eq!(hits.len(), 1, "query {query:?}");
        assert_eq!(hits[0].native_session_pk, session_pk);
        assert_eq!(hits[0].title.as_deref(), Some("Runtime Session Binding"));
        assert_eq!(hits[0].project.as_deref(), Some("Swordfish Runtime"));
        assert_eq!(hits[0].provider_id, "pi");
    }
}

#[test]
fn short_and_syntax_like_queries_are_treated_as_literal_substrings() {
    let mut registry = seed_registry();
    let matching_pk = registry
        .upsert_native_session(&native_session("native-1", "AI v1.0 notes", None))
        .expect("upsert matching session");
    replace_document(
        &mut registry,
        matching_pk,
        "literal 100%_safe and a literal \" OR * marker",
        &[],
        &[],
    );
    let other_pk = registry
        .upsert_native_session(&native_session("native-2", "Other notes", None))
        .expect("upsert other session");
    replace_document(&mut registry, other_pk, "ordinary content", &[], &[]);

    for query in ["AI", "%_", "\"", "\" OR *", "v1.0"] {
        let hits = registry.search_sessions(query, 10).expect("literal search");
        assert_eq!(hits.len(), 1, "query {query:?}");
        assert_eq!(hits[0].native_session_pk, matching_pk);
    }
    assert!(registry
        .search_sessions("anything", 0)
        .expect("zero limit")
        .is_empty());
    assert!(matches!(
        registry.search_sessions("  ", 10),
        Err(RegistryError::InvalidRecord(_))
    ));
}

#[test]
fn replacement_and_canonical_metadata_updates_keep_both_indexes_consistent() {
    let mut registry = seed_registry();
    registry
        .upsert_project(&ProjectRecord {
            id: "project-1".into(),
            display_name: "Old Project".into(),
            normalized_remotes: vec![],
            root_commit: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        })
        .expect("upsert project");
    let mut session = native_session("native-1", "Original title", Some("project-1"));
    let session_pk = registry
        .upsert_native_session(&session)
        .expect("upsert session");
    replace_document(&mut registry, session_pk, "first searchable body", &[], &[]);

    assert_eq!(
        registry
            .search_sessions("Original", 10)
            .expect("title")
            .len(),
        1
    );
    assert_eq!(
        registry
            .search_sessions("searchable", 10)
            .expect("body")
            .len(),
        1
    );

    session.title = Some("Updated title".into());
    registry
        .upsert_native_session(&session)
        .expect("update session title");
    registry
        .upsert_project(&ProjectRecord {
            id: "project-1".into(),
            display_name: "New Project".into(),
            normalized_remotes: vec![],
            root_commit: None,
            created_at_ms: 1,
            updated_at_ms: 3,
        })
        .expect("update project");
    replace_document(
        &mut registry,
        session_pk,
        "second replacement body",
        &[],
        &[],
    );

    for stale in ["Original", "Old", "searchable"] {
        assert!(registry
            .search_sessions(stale, 10)
            .expect("stale search")
            .is_empty());
    }
    for current in ["Updated", "New", "replacement"] {
        assert_eq!(
            registry
                .search_sessions(current, 10)
                .expect("current search")
                .len(),
            1
        );
    }
}

#[test]
fn migrates_version_one_rows_and_backfills_searchable_metadata() {
    let connection = Connection::open_in_memory().expect("open version one database");
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
               version INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               applied_at INTEGER NOT NULL
             );
             CREATE TABLE projects (
               id TEXT PRIMARY KEY,
               display_name TEXT NOT NULL
             );
             CREATE TABLE native_sessions (
               pk INTEGER PRIMARY KEY,
               provider_id TEXT NOT NULL,
               project_id TEXT,
               title TEXT,
               created_at INTEGER,
               updated_at INTEGER
             );
             INSERT INTO schema_migrations(version, name, applied_at)
               VALUES (1, 'canonical registry and source cursors', 1);
             INSERT INTO projects(id, display_name) VALUES ('project-1', 'Legacy Project');
             INSERT INTO native_sessions(pk, provider_id, project_id, title, created_at, updated_at)
               VALUES (7, 'claude', 'project-1', 'Legacy title', 1, 2);
             PRAGMA user_version = 1;",
        )
        .expect("seed version one schema");

    let registry = Registry::from_connection(connection).expect("migrate registry");

    assert_eq!(registry.schema_version().expect("schema version"), 2);
    let hits = registry
        .search_sessions("Legacy", 10)
        .expect("search backfill");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].native_session_pk, 7);
    assert_eq!(hits[0].project.as_deref(), Some("Legacy Project"));
}
