//! Hermes SQLite projection. Adapted from agent-sessions HermesSessionParser.swift,
//! MIT, Copyright (c) 2026 Alexander Malakhov; see fixtures/hermes/LICENSE.
use crate::{
    error::{ensure_not_cancelled, AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
    readonly_source as ro,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
pub fn database_path(root: &Path) -> PathBuf {
    root.join("state.db")
}
fn read_events(
    db: &Connection,
    id: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<PreviewEvent>> {
    let cols = ro::columns(db, "messages")?;
    let opt = |n| ro::optional(&cols, n, "NULL");
    let sql=format!("SELECT id,role,content,timestamp,{},{},{},{} FROM messages WHERE session_id=?1 ORDER BY timestamp,id",opt("tool_calls"),opt("tool_call_id"),opt("reasoning"),opt("reasoning_content"));
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    let mut out = vec![];
    while let Some(row) = rows.next()? {
        ensure_not_cancelled(cancel)?;
        let role: String = row.get(1)?;
        let content: Option<String> = row.get(2)?;
        let millis = (row.get::<_, Option<f64>>(3)?.unwrap_or(0.0) * 1000.0) as i64;
        let content = content.unwrap_or_default();
        let content_value = serde_json::from_str::<Value>(&content)
            .ok()
            .filter(Value::is_array)
            .unwrap_or(Value::String(content));
        let mut blocks = vec![];
        let reason = row.get::<_, Option<String>>(6)?.or(row.get(7)?);
        if let Some(reason) = reason {
            blocks.push(json!({"type":"thinking","thinking":reason}));
        }
        if role == "tool" {
            blocks.push(json!({"type":"tool_result","tool_use_id":row.get::<_,Option<String>>(5)?,"content":ro::text(&content_value)}));
        } else {
            let text = ro::text(&content_value);
            if !text.is_empty() {
                blocks.push(json!({"type":"text","text":text}));
            }
        }
        if let Some(calls) = row.get::<_, Option<String>>(4)? {
            let calls: Value = serde_json::from_str(&calls)?;
            if let Some(calls) = calls.as_array() {
                for call in calls {
                    blocks.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":call["function"]["arguments"]}));
                }
            }
        }
        let native = json!({"id":row.get::<_,i64>(0)?,"role":role});
        out.push(ro::event(
            out.len(),
            if role == "tool" { "user" } else { &role },
            Value::Array(blocks),
            millis,
            native,
        ));
    }
    Ok(out)
}
fn read_summary(
    db: &Connection,
    root: &Path,
    id: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<SessionSummary> {
    let cols = ro::columns(db, "sessions")?;
    let opt = |n| ro::optional(&cols, n, "NULL");
    let sql = format!(
        "SELECT started_at,ended_at,{},{},{},{} FROM sessions WHERE id=?1",
        opt("title"),
        opt("model"),
        opt("model_config"),
        opt("source")
    );
    let (created, ended, title, model, config, source) = db.query_row(&sql, [id], |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, Option<f64>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let config: Value = config
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or(Value::Null);
    let events = read_events(db, id, cancel)?;
    let last: f64 = db.query_row(
        "SELECT COALESCE(MAX(timestamp),0) FROM messages WHERE session_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let mut result = ro::summary(
        "hermes",
        id.into(),
        ro::locator("hermes", root, id),
        config["cwd"].as_str().unwrap_or("").into(),
        title.unwrap_or_default(),
        created as i64,
        ended.unwrap_or(created).max(last) as i64,
        &events,
        events.iter().map(|e| e.raw.to_string().len() as u64).sum(),
    );
    result.model = model;
    result.source = source;
    Ok(result)
}
pub fn scan(
    root: &Path,
    cancel: Option<&AtomicBool>,
    callback: &mut dyn FnMut(usize, usize, &str, AppResult<SessionSummary>),
) -> AppResult<()> {
    ensure_not_cancelled(cancel)?;
    let db = ro::open_db(root, &database_path(root))?;
    let mut stmt = db.prepare("SELECT id FROM sessions ORDER BY started_at DESC,id")?;
    let mut rows = stmt.query([])?;
    let mut ids = vec![];
    while let Some(row) = rows.next()? {
        ensure_not_cancelled(cancel)?;
        ids.push(row.get::<_, String>(0)?);
    }
    ro::register("hermes", root);
    for (i, id) in ids.iter().enumerate() {
        ensure_not_cancelled(cancel)?;
        callback(
            i + 1,
            ids.len(),
            &format!("{} · {}", database_path(root).display(), id),
            read_summary(&db, root, id, cancel),
        );
    }
    Ok(())
}
pub fn parse_session(
    root: &Path,
    locator: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    let (claimed, id) = ro::decode("hermes", locator)?;
    if claimed != root {
        return Err(AppError::Path("Hermes 来源不匹配".into()));
    }
    let db = ro::open_db(root, &database_path(root))?;
    let s = read_summary(&db, root, &id, cancel)?;
    ro::register("hermes", root);
    Ok(Some(s))
}
pub fn events(locator: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let (root, id) = ro::registered("hermes", locator)?;
    read_events(&ro::open_db(&root, &database_path(&root))?, &id, cancel)
}
pub fn preview_range(locator: &str, offset: usize, limit: usize) -> AppResult<Vec<PreviewEvent>> {
    Ok(events(locator, None)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}
pub fn preview_meta(locator: &str) -> AppResult<SessionMetaBrief> {
    let (root, id) = ro::registered("hermes", locator)?;
    Ok(ro::meta(read_summary(
        &ro::open_db(&root, &database_path(&root))?,
        &root,
        &id,
        None,
    )?))
}
