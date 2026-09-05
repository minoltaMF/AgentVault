//! Rebuildable AgentVault canonical registry.
//!
//! The registry owns only AgentVault's SQLite query projection. It never opens or mutates a
//! provider-native session file; callers supply already-discovered metadata and parsed events.

mod error;
mod model;
mod schema;
mod search;

pub use error::{RegistryError, RegistryResult};
pub use model::{
    CanonicalEvent, FileProjection, FullScanReason, MachineRecord, NativeSessionRecord,
    ProjectLocationRecord, ProjectRecord, ProjectionMode, SearchDocument, SearchHit, SourceCursor,
    SourceInstanceRecord, SourceObservation, SourceScanDecision,
};
pub use schema::LATEST_SCHEMA_VERSION;

use std::path::Path;
use std::time::Duration;

use provider_sdk::ProviderCapabilities;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::error::invalid_record;

/// SQLite-backed, rebuildable projection of provider-native sessions.
pub struct Registry {
    connection: Connection,
}

impl Registry {
    /// Open an explicitly selected registry database.
    ///
    /// Parent directories are not created implicitly, which keeps application-data placement a
    /// caller-owned policy decision.
    pub fn open(path: impl AsRef<Path>) -> RegistryResult<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Open an isolated registry suitable for tests and ephemeral projections.
    pub fn open_in_memory() -> RegistryResult<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Adopt a connection after validating or initializing the AgentVault schema.
    pub fn from_connection(mut connection: Connection) -> RegistryResult<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&mut connection)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> RegistryResult<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    pub fn foreign_keys_enabled(&self) -> RegistryResult<bool> {
        let enabled: i64 = self
            .connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        Ok(enabled == 1)
    }

    /// Return application table names for diagnostics and migration tests.
    pub fn table_names(&self) -> RegistryResult<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
               AND name NOT GLOB 'session_events_fts_*'
               AND name NOT GLOB 'session_events_trigram_*'
             ORDER BY name",
        )?;
        let names = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    pub fn upsert_machine(&mut self, machine: &MachineRecord) -> RegistryResult<()> {
        require_non_empty("machine.id", &machine.id)?;
        require_non_empty("machine.display_name", &machine.display_name)?;
        require_non_empty("machine.platform", &machine.platform)?;
        require_non_empty("machine.arch", &machine.arch)?;
        self.connection.execute(
            "INSERT INTO machines (
                 id, display_name, platform, arch, created_at, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 display_name = excluded.display_name,
                 platform = excluded.platform,
                 arch = excluded.arch,
                 created_at = MIN(machines.created_at, excluded.created_at),
                 last_seen_at = MAX(machines.last_seen_at, excluded.last_seen_at)",
            params![
                machine.id,
                machine.display_name,
                machine.platform,
                machine.arch,
                machine.observed_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_source_instance(&mut self, source: &SourceInstanceRecord) -> RegistryResult<()> {
        require_non_empty("source_instance.id", &source.id)?;
        require_non_empty("source_instance.machine_id", &source.machine_id)?;
        require_non_empty("source_instance.provider_id", &source.provider_id)?;
        require_non_empty("source_instance.config_root", &source.config_root)?;
        require_non_empty("source_instance.root_fingerprint", &source.root_fingerprint)?;
        let changed = self.connection.execute(
            "INSERT INTO source_instances (
                 id, machine_id, provider_id, config_root, root_fingerprint,
                 created_at, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 config_root = excluded.config_root,
                 created_at = MIN(source_instances.created_at, excluded.created_at),
                 last_seen_at = MAX(source_instances.last_seen_at, excluded.last_seen_at)
             WHERE source_instances.machine_id = excluded.machine_id
               AND source_instances.provider_id = excluded.provider_id
               AND source_instances.root_fingerprint = excluded.root_fingerprint",
            params![
                source.id,
                source.machine_id,
                source.provider_id,
                source.config_root,
                source.root_fingerprint,
                source.observed_at_ms,
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::IdentityConflict {
                kind: "source instance",
                id: source.id.clone(),
            });
        }
        Ok(())
    }

    pub fn upsert_project(&mut self, project: &ProjectRecord) -> RegistryResult<()> {
        require_non_empty("project.id", &project.id)?;
        require_non_empty("project.display_name", &project.display_name)?;
        if project
            .normalized_remotes
            .iter()
            .any(|remote| remote.trim().is_empty())
        {
            return Err(invalid_record(
                "project.normalized_remotes cannot contain an empty remote",
            ));
        }
        let normalized_remotes_json = serde_json::to_string(&project.normalized_remotes)?;
        self.connection.execute(
            "INSERT INTO projects (
                 id, display_name, normalized_remotes_json, root_commit, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 display_name = excluded.display_name,
                 normalized_remotes_json = excluded.normalized_remotes_json,
                 root_commit = excluded.root_commit,
                 created_at = MIN(projects.created_at, excluded.created_at),
                 updated_at = MAX(projects.updated_at, excluded.updated_at)",
            params![
                project.id,
                project.display_name,
                normalized_remotes_json,
                project.root_commit,
                project.created_at_ms,
                project.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    /// Insert or refresh one project path without rebinding its historical identity.
    pub fn upsert_project_location(
        &mut self,
        location: &ProjectLocationRecord,
    ) -> RegistryResult<()> {
        validate_project_location(location)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let path_owner: Option<String> = transaction
            .query_row(
                "SELECT id FROM project_locations WHERE machine_id = ?1 AND path = ?2",
                params![location.machine_id, location.path],
                |row| row.get(0),
            )
            .optional()?;
        if path_owner
            .as_deref()
            .is_some_and(|owner| owner != location.id)
        {
            return Err(RegistryError::IdentityConflict {
                kind: "project location",
                id: location.id.clone(),
            });
        }

        let changed = transaction.execute(
            "INSERT INTO project_locations (
                 id, project_id, machine_id, path, git_common_dir, worktree_name, branch,
                 first_seen_at, last_seen_at, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 git_common_dir = CASE
                     WHEN excluded.last_seen_at >= project_locations.last_seen_at
                     THEN excluded.git_common_dir ELSE project_locations.git_common_dir END,
                 worktree_name = CASE
                     WHEN excluded.last_seen_at >= project_locations.last_seen_at
                     THEN excluded.worktree_name ELSE project_locations.worktree_name END,
                 branch = CASE
                     WHEN excluded.last_seen_at >= project_locations.last_seen_at
                     THEN excluded.branch ELSE project_locations.branch END,
                 first_seen_at = MIN(project_locations.first_seen_at, excluded.first_seen_at),
                 last_seen_at = MAX(project_locations.last_seen_at, excluded.last_seen_at),
                 status = CASE
                     WHEN excluded.last_seen_at >= project_locations.last_seen_at
                     THEN excluded.status ELSE project_locations.status END
             WHERE project_locations.project_id = excluded.project_id
               AND project_locations.machine_id = excluded.machine_id
               AND project_locations.path = excluded.path",
            params![
                location.id,
                location.project_id,
                location.machine_id,
                location.path,
                location.git_common_dir,
                location.worktree_name,
                location.branch,
                location.first_seen_at_ms,
                location.last_seen_at_ms,
                location.status,
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::IdentityConflict {
                kind: "project location",
                id: location.id.clone(),
            });
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn project_location(
        &self,
        location_id: &str,
    ) -> RegistryResult<Option<ProjectLocationRecord>> {
        require_non_empty("project_location.id", location_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT id, project_id, machine_id, path, git_common_dir, worktree_name,
                        branch, first_seen_at, last_seen_at, status
                 FROM project_locations WHERE id = ?1",
                [location_id],
                project_location_from_row,
            )
            .optional()?)
    }

    /// List path history newest-first for one canonical project.
    pub fn project_locations(
        &self,
        project_id: &str,
    ) -> RegistryResult<Vec<ProjectLocationRecord>> {
        require_non_empty("project_location.project_id", project_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, machine_id, path, git_common_dir, worktree_name,
                    branch, first_seen_at, last_seen_at, status
             FROM project_locations
             WHERE project_id = ?1
             ORDER BY last_seen_at DESC, id",
        )?;
        let locations = statement
            .query_map([project_id], project_location_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(locations)
    }

    /// Insert or refresh one native session while preserving its composite native identity.
    pub fn upsert_native_session(&mut self, session: &NativeSessionRecord) -> RegistryResult<i64> {
        validate_native_session(session)?;
        let metadata_json = serde_json::to_string(&session.metadata)?;
        let capabilities = session.capabilities.bits() as i64;
        self.connection.execute(
            "INSERT INTO native_sessions (
                 machine_id, source_instance_id, provider_id, native_session_id, project_id,
                 root_native_session_id, parent_native_session_id, title, cwd_at_start, model,
                 created_at, updated_at, source_format_version, parser_version, health_status,
                 resumability, capabilities, metadata_json
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )
             ON CONFLICT(machine_id, source_instance_id, provider_id, native_session_id)
             DO UPDATE SET
                 project_id = excluded.project_id,
                 root_native_session_id = excluded.root_native_session_id,
                 parent_native_session_id = excluded.parent_native_session_id,
                 title = excluded.title,
                 cwd_at_start = excluded.cwd_at_start,
                 model = excluded.model,
                 created_at = excluded.created_at,
                 updated_at = excluded.updated_at,
                 source_format_version = excluded.source_format_version,
                 parser_version = excluded.parser_version,
                 health_status = excluded.health_status,
                 resumability = excluded.resumability,
                 capabilities = excluded.capabilities,
                 metadata_json = excluded.metadata_json",
            params![
                session.machine_id,
                session.source_instance_id,
                session.provider_id,
                session.native_session_id,
                session.project_id,
                session.root_native_session_id,
                session.parent_native_session_id,
                session.title,
                session.cwd_at_start,
                session.model,
                session.created_at_ms,
                session.updated_at_ms,
                session.source_format_version,
                session.parser_version,
                session.health_status,
                session.resumability,
                capabilities,
                metadata_json,
            ],
        )?;
        Ok(self.connection.query_row(
            "SELECT pk FROM native_sessions
             WHERE machine_id = ?1 AND source_instance_id = ?2
               AND provider_id = ?3 AND native_session_id = ?4",
            params![
                session.machine_id,
                session.source_instance_id,
                session.provider_id,
                session.native_session_id,
            ],
            |row| row.get(0),
        )?)
    }

    pub fn native_session(&self, session_pk: i64) -> RegistryResult<Option<NativeSessionRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT machine_id, source_instance_id, provider_id, native_session_id,
                        project_id, root_native_session_id, parent_native_session_id, title,
                        cwd_at_start, model, created_at, updated_at, source_format_version,
                        parser_version, health_status, resumability, capabilities, metadata_json
                 FROM native_sessions WHERE pk = ?1",
                [session_pk],
                |row| {
                    let capabilities: i64 = row.get(16)?;
                    let metadata_json: String = row.get(17)?;
                    Ok((
                        NativeSessionRecord {
                            machine_id: row.get(0)?,
                            source_instance_id: row.get(1)?,
                            provider_id: row.get(2)?,
                            native_session_id: row.get(3)?,
                            project_id: row.get(4)?,
                            root_native_session_id: row.get(5)?,
                            parent_native_session_id: row.get(6)?,
                            title: row.get(7)?,
                            cwd_at_start: row.get(8)?,
                            model: row.get(9)?,
                            created_at_ms: row.get(10)?,
                            updated_at_ms: row.get(11)?,
                            source_format_version: row.get(12)?,
                            parser_version: row.get(13)?,
                            health_status: row.get(14)?,
                            resumability: row.get(15)?,
                            capabilities: ProviderCapabilities::from_bits_retain(
                                capabilities as u64,
                            ),
                            metadata: serde_json::Value::Null,
                        },
                        metadata_json,
                    ))
                },
            )
            .optional()?;
        stored
            .map(|(mut record, metadata_json)| {
                record.metadata = serde_json::from_str(&metadata_json)?;
                Ok(record)
            })
            .transpose()
    }

    pub fn native_session_count(&self) -> RegistryResult<u64> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM native_sessions", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Decide whether a source file is unchanged, safely appendable, or must be rebuilt.
    pub fn source_scan_decision(
        &self,
        native_session_pk: i64,
        absolute_path: &str,
        observation: &SourceObservation,
        parser_version: &str,
    ) -> RegistryResult<SourceScanDecision> {
        require_non_empty("source_file.absolute_path", absolute_path)?;
        require_non_empty("source_file.parser_version", parser_version)?;
        let Some(cursor) = load_source_cursor(&self.connection, native_session_pk, absolute_path)?
        else {
            return Ok(SourceScanDecision::FullScan(FullScanReason::MissingCursor));
        };
        Ok(scan_decision(&cursor, observation, parser_version))
    }

    pub fn source_cursor(
        &self,
        native_session_pk: i64,
        absolute_path: &str,
    ) -> RegistryResult<Option<SourceCursor>> {
        load_source_cursor(&self.connection, native_session_pk, absolute_path)
    }

    /// Commit canonical events and their source cursor as one SQLite transaction.
    ///
    /// `Rebuild` replaces only the projection derived from this source file. `Append` requires a
    /// compatible prior cursor and is rejected when identity, parser version, size, or offsets
    /// indicate that a full rebuild is necessary.
    pub fn commit_file_projection(&mut self, projection: &FileProjection) -> RegistryResult<()> {
        validate_projection(projection)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let session_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_sessions WHERE pk = ?1)",
            [projection.native_session_pk],
            |row| row.get(0),
        )?;
        if !session_exists {
            return Err(RegistryError::MissingRecord {
                kind: "native session",
                id: projection.native_session_pk.to_string(),
            });
        }

        let previous = load_source_cursor(
            &transaction,
            projection.native_session_pk,
            &projection.absolute_path,
        )?;
        if projection.mode == ProjectionMode::Append {
            let previous = previous.ok_or(RegistryError::CursorRequiresRebuild(
                FullScanReason::MissingCursor,
            ))?;
            validate_append_transition(
                &previous,
                &projection.cursor,
                projection.verified_previous_hash.as_deref(),
            )?;
        }

        transaction.execute(
            "INSERT INTO source_files (
                 native_session_pk, role, absolute_path, file_identity, size, mtime_ns,
                 parsed_offset, last_complete_line_offset, trailing_partial, parser_version,
                 last_hash, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(native_session_pk, absolute_path) DO UPDATE SET
                 role = excluded.role,
                 file_identity = excluded.file_identity,
                 size = excluded.size,
                 mtime_ns = excluded.mtime_ns,
                 parsed_offset = excluded.parsed_offset,
                 last_complete_line_offset = excluded.last_complete_line_offset,
                 trailing_partial = excluded.trailing_partial,
                 parser_version = excluded.parser_version,
                 last_hash = excluded.last_hash,
                 last_seen_at = excluded.last_seen_at",
            params![
                projection.native_session_pk,
                projection.role,
                projection.absolute_path,
                projection.cursor.file_identity,
                to_sql_integer("source_cursor.size", projection.cursor.size)?,
                projection.cursor.mtime_ns,
                to_sql_integer(
                    "source_cursor.parsed_offset",
                    projection.cursor.parsed_offset,
                )?,
                to_sql_integer(
                    "source_cursor.last_complete_line_offset",
                    projection.cursor.last_complete_line_offset,
                )?,
                projection.cursor.partial_tail,
                projection.cursor.parser_version,
                projection.cursor.last_hash,
                projection.cursor.last_seen_at_ms,
            ],
        )?;
        let source_file_id: i64 = transaction.query_row(
            "SELECT id FROM source_files
             WHERE native_session_pk = ?1 AND absolute_path = ?2",
            params![projection.native_session_pk, projection.absolute_path],
            |row| row.get(0),
        )?;

        if projection.mode == ProjectionMode::Rebuild {
            transaction.execute(
                "DELETE FROM session_events WHERE raw_source_file_id = ?1",
                [source_file_id],
            )?;
        }
        for event in &projection.events {
            let structured_json = serde_json::to_string(&event.structured)?;
            let raw_byte_start = event
                .raw_byte_start
                .map(|value| to_sql_integer("canonical_event.raw_byte_start", value))
                .transpose()?;
            let raw_byte_end = event
                .raw_byte_end
                .map(|value| to_sql_integer("canonical_event.raw_byte_end", value))
                .transpose()?;
            transaction.execute(
                "INSERT INTO session_events (
                     native_session_pk, branch_id, event_id, native_event_id, parent_event_id,
                     ordinal, timestamp, kind, role, plain_text, tool_call_id, structured_json,
                     raw_source_file_id, raw_byte_start, raw_byte_end, parse_quality
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
                 )
                 ON CONFLICT(native_session_pk, event_id) DO UPDATE SET
                     branch_id = excluded.branch_id,
                     native_event_id = excluded.native_event_id,
                     parent_event_id = excluded.parent_event_id,
                     ordinal = excluded.ordinal,
                     timestamp = excluded.timestamp,
                     kind = excluded.kind,
                     role = excluded.role,
                     plain_text = excluded.plain_text,
                     tool_call_id = excluded.tool_call_id,
                     structured_json = excluded.structured_json,
                     raw_source_file_id = excluded.raw_source_file_id,
                     raw_byte_start = excluded.raw_byte_start,
                     raw_byte_end = excluded.raw_byte_end,
                     parse_quality = excluded.parse_quality",
                params![
                    projection.native_session_pk,
                    event.branch_id,
                    event.event_id,
                    event.native_event_id,
                    event.parent_event_id,
                    event.ordinal,
                    event.timestamp_ms,
                    event.kind,
                    event.role,
                    event.plain_text,
                    event.tool_call_id,
                    structured_json,
                    source_file_id,
                    raw_byte_start,
                    raw_byte_end,
                    event.parse_quality,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn events_for_session(
        &self,
        native_session_pk: i64,
    ) -> RegistryResult<Vec<CanonicalEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT event_id, branch_id, native_event_id, parent_event_id, ordinal, timestamp,
                    kind, role, plain_text, tool_call_id, structured_json, raw_byte_start, raw_byte_end,
                    parse_quality
             FROM session_events
             WHERE native_session_pk = ?1
             ORDER BY ordinal, event_id",
        )?;
        let rows = statement.query_map([native_session_pk], |row| {
            let structured_json: String = row.get(10)?;
            let raw_byte_start: Option<i64> = row.get(11)?;
            let raw_byte_end: Option<i64> = row.get(12)?;
            Ok((
                CanonicalEvent {
                    event_id: row.get(0)?,
                    branch_id: row.get(1)?,
                    native_event_id: row.get(2)?,
                    parent_event_id: row.get(3)?,
                    ordinal: row.get(4)?,
                    timestamp_ms: row.get(5)?,
                    kind: row.get(6)?,
                    role: row.get(7)?,
                    plain_text: row.get(8)?,
                    tool_call_id: row.get(9)?,
                    structured: serde_json::Value::Null,
                    raw_byte_start: raw_byte_start.map(|value| value as u64),
                    raw_byte_end: raw_byte_end.map(|value| value as u64),
                    parse_quality: row.get(13)?,
                },
                structured_json,
            ))
        })?;
        rows.map(|row| {
            let (mut event, structured_json) = row?;
            event.structured = serde_json::from_str(&structured_json)?;
            Ok(event)
        })
        .collect()
    }

    /// Replace the explicitly searchable text for one canonical session.
    pub fn replace_search_document(&mut self, document: &SearchDocument) -> RegistryResult<()> {
        search::replace_document(&mut self.connection, document)
    }

    /// Search canonical sessions without interpreting the input as FTS query syntax.
    ///
    /// Queries shorter than three Unicode scalar values use a literal substring fallback because
    /// the trigram tokenizer cannot match them. Ranking and filters are intentionally left to the
    /// next search-layer change.
    pub fn search_sessions(&self, query: &str, limit: u32) -> RegistryResult<Vec<SearchHit>> {
        search::search_sessions(&self.connection, query, limit)
    }
}

fn load_source_cursor(
    connection: &Connection,
    native_session_pk: i64,
    absolute_path: &str,
) -> RegistryResult<Option<SourceCursor>> {
    let cursor = connection
        .query_row(
            "SELECT file_identity, size, mtime_ns, parsed_offset, last_complete_line_offset,
                    trailing_partial, parser_version, last_hash, last_seen_at
             FROM source_files
             WHERE native_session_pk = ?1 AND absolute_path = ?2",
            params![native_session_pk, absolute_path],
            |row| {
                let size: i64 = row.get(1)?;
                let parsed_offset: i64 = row.get(3)?;
                let last_complete_line_offset: i64 = row.get(4)?;
                Ok(SourceCursor {
                    file_identity: row.get(0)?,
                    size: size as u64,
                    mtime_ns: row.get(2)?,
                    parsed_offset: parsed_offset as u64,
                    last_complete_line_offset: last_complete_line_offset as u64,
                    partial_tail: row.get(5)?,
                    parser_version: row.get(6)?,
                    last_hash: row.get(7)?,
                    last_seen_at_ms: row.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(cursor)
}

fn scan_decision(
    cursor: &SourceCursor,
    observation: &SourceObservation,
    parser_version: &str,
) -> SourceScanDecision {
    if cursor.parser_version != parser_version {
        return SourceScanDecision::FullScan(FullScanReason::ParserVersionChanged);
    }
    match (&cursor.file_identity, &observation.file_identity) {
        (Some(previous), Some(current)) if previous != current => {
            return SourceScanDecision::FullScan(FullScanReason::FileReplaced);
        }
        (None, None) => {
            if cursor.size == observation.size
                && cursor.mtime_ns == observation.mtime_ns
                && observation.verified_last_hash.as_deref() == Some(cursor.last_hash.as_str())
            {
                return SourceScanDecision::Unchanged;
            }
            return SourceScanDecision::FullScan(FullScanReason::IdentityUnavailable);
        }
        (Some(_), None) | (None, Some(_)) => {
            return SourceScanDecision::FullScan(FullScanReason::IdentityUnavailable);
        }
        (Some(_), Some(_)) => {}
    }
    if observation.size < cursor.size {
        return SourceScanDecision::FullScan(FullScanReason::Truncated);
    }
    if observation.size == cursor.size {
        match (
            cursor.last_hash.as_str(),
            observation.verified_last_hash.as_deref(),
        ) {
            (_, None) => {
                return SourceScanDecision::FullScan(FullScanReason::HashUnverified);
            }
            (previous, Some(current)) if previous != current => {
                return SourceScanDecision::FullScan(FullScanReason::ContentChanged);
            }
            _ => {}
        }
        if observation.mtime_ns == cursor.mtime_ns {
            SourceScanDecision::Unchanged
        } else {
            SourceScanDecision::FullScan(FullScanReason::ContentChanged)
        }
    } else {
        match (
            cursor.last_hash.as_str(),
            observation.verified_last_hash.as_deref(),
        ) {
            (previous, Some(current)) if previous == current => SourceScanDecision::Resume {
                parsed_offset: cursor.parsed_offset,
                partial_tail: cursor.partial_tail.clone(),
            },
            (_, Some(_)) => SourceScanDecision::FullScan(FullScanReason::ContentChanged),
            (_, None) => SourceScanDecision::FullScan(FullScanReason::HashUnverified),
        }
    }
}

fn validate_append_transition(
    previous: &SourceCursor,
    next: &SourceCursor,
    verified_previous_hash: Option<&str>,
) -> RegistryResult<()> {
    let observation = SourceObservation {
        file_identity: next.file_identity.clone(),
        size: next.size,
        mtime_ns: next.mtime_ns,
        verified_last_hash: verified_previous_hash.map(str::to_owned),
    };
    match scan_decision(previous, &observation, &next.parser_version) {
        SourceScanDecision::FullScan(reason) => {
            return Err(RegistryError::CursorRequiresRebuild(reason));
        }
        SourceScanDecision::Unchanged | SourceScanDecision::Resume { .. } => {}
    }
    if next.parsed_offset < previous.parsed_offset {
        return Err(invalid_record(
            "source_cursor.parsed_offset cannot move backwards during append",
        ));
    }
    if next.last_complete_line_offset < previous.last_complete_line_offset {
        return Err(invalid_record(
            "source_cursor.last_complete_line_offset cannot move backwards during append",
        ));
    }
    Ok(())
}

fn validate_native_session(session: &NativeSessionRecord) -> RegistryResult<()> {
    require_non_empty("native_session.machine_id", &session.machine_id)?;
    require_non_empty(
        "native_session.source_instance_id",
        &session.source_instance_id,
    )?;
    require_non_empty("native_session.provider_id", &session.provider_id)?;
    require_non_empty(
        "native_session.native_session_id",
        &session.native_session_id,
    )?;
    require_non_empty("native_session.parser_version", &session.parser_version)?;
    require_non_empty("native_session.health_status", &session.health_status)?;
    require_non_empty("native_session.resumability", &session.resumability)?;
    if !session.metadata.is_object() {
        return Err(invalid_record(
            "native_session.metadata must be a JSON object",
        ));
    }
    Ok(())
}

fn validate_project_location(location: &ProjectLocationRecord) -> RegistryResult<()> {
    require_non_empty("project_location.id", &location.id)?;
    require_non_empty("project_location.project_id", &location.project_id)?;
    require_non_empty("project_location.machine_id", &location.machine_id)?;
    require_non_empty("project_location.path", &location.path)?;
    require_non_empty("project_location.status", &location.status)?;
    if location.first_seen_at_ms > location.last_seen_at_ms {
        return Err(invalid_record(
            "project_location.first_seen_at_ms cannot exceed last_seen_at_ms",
        ));
    }
    Ok(())
}

fn project_location_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectLocationRecord> {
    Ok(ProjectLocationRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        machine_id: row.get(2)?,
        path: row.get(3)?,
        git_common_dir: row.get(4)?,
        worktree_name: row.get(5)?,
        branch: row.get(6)?,
        first_seen_at_ms: row.get(7)?,
        last_seen_at_ms: row.get(8)?,
        status: row.get(9)?,
    })
}

fn validate_projection(projection: &FileProjection) -> RegistryResult<()> {
    require_non_empty("source_file.role", &projection.role)?;
    require_non_empty("source_file.absolute_path", &projection.absolute_path)?;
    validate_cursor(&projection.cursor)?;
    for event in &projection.events {
        require_non_empty("canonical_event.event_id", &event.event_id)?;
        require_non_empty("canonical_event.kind", &event.kind)?;
        require_non_empty("canonical_event.parse_quality", &event.parse_quality)?;
        if event.ordinal < 0 {
            return Err(invalid_record(
                "canonical_event.ordinal must be non-negative",
            ));
        }
        match (event.raw_byte_start, event.raw_byte_end) {
            (None, None) => {}
            (Some(start), Some(end)) if start <= end && end <= projection.cursor.parsed_offset => {}
            (Some(_), Some(_)) => {
                return Err(invalid_record(
                    "canonical_event raw byte range must be ordered and already parsed",
                ));
            }
            _ => {
                return Err(invalid_record(
                    "canonical_event raw byte range requires both start and end",
                ));
            }
        }
    }
    Ok(())
}

fn validate_cursor(cursor: &SourceCursor) -> RegistryResult<()> {
    require_non_empty("source_cursor.parser_version", &cursor.parser_version)?;
    require_non_empty("source_cursor.last_hash", &cursor.last_hash)?;
    to_sql_integer("source_cursor.size", cursor.size)?;
    to_sql_integer("source_cursor.parsed_offset", cursor.parsed_offset)?;
    to_sql_integer(
        "source_cursor.last_complete_line_offset",
        cursor.last_complete_line_offset,
    )?;
    if cursor.parsed_offset > cursor.size {
        return Err(invalid_record(
            "source_cursor.parsed_offset cannot exceed size",
        ));
    }
    if cursor.parsed_offset != cursor.size {
        return Err(invalid_record(
            "source_cursor.parsed_offset must cover the observed file size",
        ));
    }
    if cursor.last_complete_line_offset > cursor.parsed_offset {
        return Err(invalid_record(
            "source_cursor.last_complete_line_offset cannot exceed parsed_offset",
        ));
    }
    let expected_tail = cursor.parsed_offset - cursor.last_complete_line_offset;
    if expected_tail != cursor.partial_tail.len() as u64 {
        return Err(invalid_record(
            "source_cursor.partial_tail length must match the incomplete byte range",
        ));
    }
    Ok(())
}

fn require_non_empty(field: &str, value: &str) -> RegistryResult<()> {
    if value.trim().is_empty() {
        Err(invalid_record(format!("{field} cannot be empty")))
    } else {
        Ok(())
    }
}

fn to_sql_integer(field: &str, value: u64) -> RegistryResult<i64> {
    i64::try_from(value).map_err(|_| invalid_record(format!("{field} exceeds SQLite INTEGER")))
}
