use rusqlite::{params, Connection, TransactionBehavior};

use crate::{RegistryError, RegistryResult, SearchDocument, SearchHit};

pub(crate) fn replace_document(
    connection: &mut Connection,
    document: &SearchDocument,
) -> RegistryResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "INSERT INTO session_search_projection (
             rowid, title, project, provider, content, tool_names, paths
         )
         SELECT native_sessions.pk,
                COALESCE(native_sessions.title, ''),
                COALESCE(projects.display_name, ''),
                native_sessions.provider_id,
                ?2, ?3, ?4
         FROM native_sessions
         LEFT JOIN projects ON projects.id = native_sessions.project_id
         WHERE native_sessions.pk = ?1
         ON CONFLICT(rowid) DO UPDATE SET
             title = excluded.title,
             project = excluded.project,
             provider = excluded.provider,
             content = excluded.content,
             tool_names = excluded.tool_names,
             paths = excluded.paths",
        params![
            document.native_session_pk,
            document.content,
            document.tool_names.join("\n"),
            document.paths.join("\n"),
        ],
    )?;
    if changed == 0 {
        return Err(RegistryError::MissingRecord {
            kind: "native session",
            id: document.native_session_pk.to_string(),
        });
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn search_sessions(
    connection: &Connection,
    query: &str,
    limit: u32,
) -> RegistryResult<Vec<SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Err(RegistryError::InvalidRecord(
            "search query cannot be empty".into(),
        ));
    }
    if limit == 0 {
        return Ok(Vec::new());
    }

    if query.chars().count() < 3 {
        search_short_literal(connection, query, limit)
    } else {
        search_fts(connection, query, limit)
    }
}

fn search_short_literal(
    connection: &Connection,
    query: &str,
    limit: u32,
) -> RegistryResult<Vec<SearchHit>> {
    let mut statement = connection.prepare(
        "SELECT projection.rowid, sessions.title,
                NULLIF(projection.project, ''), sessions.provider_id
         FROM session_search_projection AS projection
         JOIN native_sessions AS sessions ON sessions.pk = projection.rowid
         WHERE instr(lower(projection.title), lower(?1)) > 0
            OR instr(lower(projection.project), lower(?1)) > 0
            OR instr(lower(projection.provider), lower(?1)) > 0
            OR instr(lower(projection.content), lower(?1)) > 0
            OR instr(lower(projection.tool_names), lower(?1)) > 0
            OR instr(lower(projection.paths), lower(?1)) > 0
         ORDER BY COALESCE(sessions.updated_at, sessions.created_at, 0) DESC,
                  projection.rowid
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![query, i64::from(limit)], map_hit)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn search_fts(connection: &Connection, query: &str, limit: u32) -> RegistryResult<Vec<SearchHit>> {
    let escaped = query.replace('"', "\"\"");
    let trigram_query = format!("\"{escaped}\"");
    if query
        .chars()
        .all(|character| character.is_alphanumeric() || character.is_whitespace())
    {
        return search_unicode_and_trigram(
            connection,
            &format!("\"{escaped}\" *"),
            &trigram_query,
            limit,
        );
    }

    let mut statement = connection.prepare(
        "WITH matches(rowid) AS (
             SELECT rowid FROM session_events_trigram
             WHERE session_events_trigram MATCH ?1
             UNION
             SELECT rowid FROM session_search_projection
             WHERE instr(lower(title), lower(?2)) > 0
                OR instr(lower(project), lower(?2)) > 0
                OR instr(lower(provider), lower(?2)) > 0
                OR instr(lower(tool_names), lower(?2)) > 0
         )
         SELECT matches.rowid, sessions.title,
                NULLIF(projection.project, ''), sessions.provider_id
         FROM matches
         JOIN session_search_projection AS projection ON projection.rowid = matches.rowid
         JOIN native_sessions AS sessions ON sessions.pk = matches.rowid
         ORDER BY COALESCE(sessions.updated_at, sessions.created_at, 0) DESC,
                  matches.rowid
         LIMIT ?3",
    )?;
    let rows = statement.query_map(params![trigram_query, query, i64::from(limit)], map_hit)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn search_unicode_and_trigram(
    connection: &Connection,
    unicode_query: &str,
    trigram_query: &str,
    limit: u32,
) -> RegistryResult<Vec<SearchHit>> {
    let mut statement = connection.prepare(
        "WITH matches(rowid) AS (
             SELECT rowid FROM session_events_fts
             WHERE session_events_fts MATCH ?1
             UNION
             SELECT rowid FROM session_events_trigram
             WHERE session_events_trigram MATCH ?2
         )
         SELECT matches.rowid, sessions.title,
                NULLIF(projection.project, ''), sessions.provider_id
         FROM matches
         JOIN native_sessions AS sessions ON sessions.pk = matches.rowid
         JOIN session_search_projection AS projection ON projection.rowid = matches.rowid
         ORDER BY COALESCE(sessions.updated_at, sessions.created_at, 0) DESC,
                  matches.rowid
         LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![unicode_query, trigram_query, i64::from(limit)],
        map_hit,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn map_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        native_session_pk: row.get(0)?,
        title: row.get(1)?,
        project: row.get(2)?,
        provider_id: row.get(3)?,
    })
}
