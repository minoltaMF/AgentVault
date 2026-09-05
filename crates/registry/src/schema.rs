use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{RegistryError, RegistryResult};

pub const LATEST_SCHEMA_VERSION: i64 = 2;

const MIGRATION_1: &str = r#"
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  applied_at INTEGER NOT NULL
);

CREATE TABLE machines (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  platform TEXT NOT NULL,
  arch TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL
);

CREATE TABLE source_instances (
  id TEXT PRIMARY KEY,
  machine_id TEXT NOT NULL REFERENCES machines(id),
  provider_id TEXT NOT NULL,
  config_root TEXT NOT NULL,
  root_fingerprint TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  UNIQUE(machine_id, provider_id, root_fingerprint),
  UNIQUE(id, machine_id, provider_id)
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  normalized_remotes_json TEXT,
  root_commit TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE project_locations (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  machine_id TEXT NOT NULL REFERENCES machines(id),
  path TEXT NOT NULL,
  git_common_dir TEXT,
  worktree_name TEXT,
  branch TEXT,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  status TEXT NOT NULL,
  UNIQUE(machine_id, path)
);

CREATE TABLE native_sessions (
  pk INTEGER PRIMARY KEY,
  machine_id TEXT NOT NULL,
  source_instance_id TEXT NOT NULL,
  provider_id TEXT NOT NULL,
  native_session_id TEXT NOT NULL,
  project_id TEXT REFERENCES projects(id),
  root_native_session_id TEXT,
  parent_native_session_id TEXT,
  title TEXT,
  cwd_at_start TEXT,
  model TEXT,
  created_at INTEGER,
  updated_at INTEGER,
  source_format_version TEXT,
  parser_version TEXT NOT NULL,
  health_status TEXT NOT NULL,
  resumability TEXT NOT NULL,
  capabilities INTEGER NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  FOREIGN KEY(source_instance_id, machine_id, provider_id)
    REFERENCES source_instances(id, machine_id, provider_id),
  UNIQUE(machine_id, source_instance_id, provider_id, native_session_id)
);

CREATE TABLE source_files (
  id INTEGER PRIMARY KEY,
  native_session_pk INTEGER NOT NULL REFERENCES native_sessions(pk) ON DELETE CASCADE,
  role TEXT NOT NULL,
  absolute_path TEXT NOT NULL,
  file_identity TEXT,
  size INTEGER NOT NULL CHECK(size >= 0),
  mtime_ns INTEGER NOT NULL,
  parsed_offset INTEGER NOT NULL DEFAULT 0 CHECK(parsed_offset >= 0),
  last_complete_line_offset INTEGER NOT NULL DEFAULT 0
    CHECK(last_complete_line_offset >= 0),
  trailing_partial BLOB NOT NULL DEFAULT X'',
  parser_version TEXT NOT NULL,
  last_hash TEXT NOT NULL,
  last_seen_at INTEGER NOT NULL,
  CHECK(last_complete_line_offset <= parsed_offset),
  CHECK(parsed_offset <= size),
  CHECK(length(trailing_partial) = parsed_offset - last_complete_line_offset),
  UNIQUE(native_session_pk, absolute_path)
);

CREATE TABLE session_events (
  pk INTEGER PRIMARY KEY,
  native_session_pk INTEGER NOT NULL REFERENCES native_sessions(pk) ON DELETE CASCADE,
  branch_id TEXT,
  event_id TEXT NOT NULL,
  native_event_id TEXT,
  parent_event_id TEXT,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  timestamp INTEGER,
  kind TEXT NOT NULL,
  role TEXT,
  plain_text TEXT,
  tool_call_id TEXT,
  structured_json TEXT NOT NULL,
  raw_source_file_id INTEGER REFERENCES source_files(id) ON DELETE CASCADE,
  raw_byte_start INTEGER,
  raw_byte_end INTEGER,
  parse_quality TEXT NOT NULL,
  CHECK(
    (raw_byte_start IS NULL AND raw_byte_end IS NULL)
    OR (raw_byte_start >= 0 AND raw_byte_end >= raw_byte_start)
  ),
  UNIQUE(native_session_pk, event_id)
);

CREATE INDEX source_files_session_idx ON source_files(native_session_pk);
CREATE INDEX session_events_session_ordinal_idx
  ON session_events(native_session_pk, ordinal, event_id);
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE session_search_projection (
  rowid INTEGER PRIMARY KEY REFERENCES native_sessions(pk) ON DELETE CASCADE,
  title TEXT NOT NULL DEFAULT '',
  project TEXT NOT NULL DEFAULT '',
  provider TEXT NOT NULL,
  content TEXT NOT NULL DEFAULT '',
  tool_names TEXT NOT NULL DEFAULT '',
  paths TEXT NOT NULL DEFAULT ''
);

CREATE VIRTUAL TABLE session_events_fts USING fts5(
  title,
  project,
  provider,
  content,
  tool_names,
  paths,
  content='session_search_projection',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2',
  prefix='2 3'
);

CREATE VIRTUAL TABLE session_events_trigram USING fts5(
  content,
  paths,
  content='session_search_projection',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE TRIGGER session_search_projection_ai
AFTER INSERT ON session_search_projection BEGIN
  INSERT INTO session_events_fts(
    rowid, title, project, provider, content, tool_names, paths
  ) VALUES (
    new.rowid, new.title, new.project, new.provider, new.content, new.tool_names, new.paths
  );
  INSERT INTO session_events_trigram(rowid, content, paths)
  VALUES (new.rowid, new.content, new.paths);
END;

CREATE TRIGGER session_search_projection_ad
AFTER DELETE ON session_search_projection BEGIN
  INSERT INTO session_events_fts(
    session_events_fts, rowid, title, project, provider, content, tool_names, paths
  ) VALUES (
    'delete', old.rowid, old.title, old.project, old.provider, old.content,
    old.tool_names, old.paths
  );
  INSERT INTO session_events_trigram(
    session_events_trigram, rowid, content, paths
  ) VALUES ('delete', old.rowid, old.content, old.paths);
END;

CREATE TRIGGER session_search_projection_au
AFTER UPDATE ON session_search_projection BEGIN
  INSERT INTO session_events_fts(
    session_events_fts, rowid, title, project, provider, content, tool_names, paths
  ) VALUES (
    'delete', old.rowid, old.title, old.project, old.provider, old.content,
    old.tool_names, old.paths
  );
  INSERT INTO session_events_fts(
    rowid, title, project, provider, content, tool_names, paths
  ) VALUES (
    new.rowid, new.title, new.project, new.provider, new.content, new.tool_names, new.paths
  );
  INSERT INTO session_events_trigram(
    session_events_trigram, rowid, content, paths
  ) VALUES ('delete', old.rowid, old.content, old.paths);
  INSERT INTO session_events_trigram(rowid, content, paths)
  VALUES (new.rowid, new.content, new.paths);
END;

CREATE TRIGGER native_sessions_search_ai
AFTER INSERT ON native_sessions BEGIN
  INSERT INTO session_search_projection(
    rowid, title, project, provider, content, tool_names, paths
  ) VALUES (
    new.pk,
    COALESCE(new.title, ''),
    COALESCE((SELECT display_name FROM projects WHERE id = new.project_id), ''),
    new.provider_id,
    '', '', ''
  );
END;

CREATE TRIGGER native_sessions_search_au
AFTER UPDATE OF title, project_id, provider_id ON native_sessions BEGIN
  UPDATE session_search_projection
  SET title = COALESCE(new.title, ''),
      project = COALESCE(
        (SELECT display_name FROM projects WHERE id = new.project_id),
        ''
      ),
      provider = new.provider_id
  WHERE rowid = new.pk;
END;

CREATE TRIGGER projects_search_au
AFTER UPDATE OF display_name ON projects BEGIN
  UPDATE session_search_projection
  SET project = new.display_name
  WHERE rowid IN (
    SELECT pk FROM native_sessions WHERE project_id = new.id
  );
END;

INSERT INTO session_search_projection(
  rowid, title, project, provider, content, tool_names, paths
)
SELECT native_sessions.pk,
       COALESCE(native_sessions.title, ''),
       COALESCE(projects.display_name, ''),
       native_sessions.provider_id,
       '', '', ''
FROM native_sessions
LEFT JOIN projects ON projects.id = native_sessions.project_id;
"#;

pub(crate) fn migrate(connection: &mut Connection) -> RegistryResult<()> {
    let mut version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > LATEST_SCHEMA_VERSION {
        return Err(RegistryError::SchemaTooNew {
            found: version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    if version == 0 {
        if has_application_tables(connection)? {
            return Err(RegistryError::ConflictingSchema);
        }
        apply_migration(
            connection,
            1,
            "canonical registry and source cursors",
            MIGRATION_1,
        )?;
        version = 1;
    } else {
        validate_recorded_version(connection, version)?;
    }

    if version == 1 {
        apply_migration(connection, 2, "unicode61 and trigram search", MIGRATION_2)?;
        version = 2;
    }

    validate_recorded_version(connection, version)
}

fn apply_migration(
    connection: &mut Connection,
    version: i64,
    name: &str,
    sql: &str,
) -> RegistryResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(sql)?;
    transaction.execute(
        "INSERT INTO schema_migrations(version, name, applied_at)
         VALUES (?1, ?2, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
        params![version, name],
    )?;
    transaction.pragma_update(None, "user_version", version)?;
    transaction.commit()?;
    Ok(())
}

fn validate_recorded_version(connection: &Connection, version: i64) -> RegistryResult<()> {
    let recorded: Option<i64> = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .optional()?
        .flatten();
    if recorded != Some(version) {
        return Err(RegistryError::InconsistentSchema(format!(
            "PRAGMA user_version is {version}, but schema_migrations records {recorded:?}"
        )));
    }
    Ok(())
}

fn has_application_tables(connection: &Connection) -> RegistryResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         )",
        [],
        |row| row.get(0),
    )?)
}
