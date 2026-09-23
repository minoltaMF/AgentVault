//! Read-only ZCode SQLite projection, adapted from zai-org/ZCode's session-store
//! at 872ad960de7ec172591f7e1952f7849229f94521 (Apache-2.0). See fixtures/zcode/LICENSE.
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
    root.join("cli/db/db.sqlite")
}
fn read_events(
    db: &Connection,
    id: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<PreviewEvent>> {
    let mcols = ro::columns(db, "message")?;
    let pcols = ro::columns(db, "part")?;
    let mseq = if mcols.contains("sequence") {
        "m.sequence IS NULL,m.sequence,"
    } else {
        ""
    };
    let pseq = if pcols.contains("sequence") {
        "p.sequence IS NULL,p.sequence,"
    } else {
        ""
    };
    let sql=format!("SELECT m.id,m.data,p.id,p.data,p.time_created FROM message m JOIN part p ON p.message_id=m.id AND p.session_id=m.session_id WHERE m.session_id=?1 ORDER BY {mseq}m.time_created,m.rowid,{pseq}p.time_created,p.id");
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    let mut out = vec![];
    while let Some(row) = rows.next()? {
        ensure_not_cancelled(cancel)?;
        let message: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let part: Value = serde_json::from_str(&row.get::<_, String>(3)?)?;
        let role = message["role"]
            .as_str()
            .ok_or_else(|| AppError::Other("ZCode message 缺少 role".into()))?;
        let millis = row.get::<_, i64>(4)?;
        let block = match part["type"].as_str() {
            Some("text") => json!({"type":"text","text":part["text"]}),
            Some("reasoning") => json!({"type":"thinking","thinking":part["text"]}),
            Some("tool") => {
                json!({"type":"tool_use","id":part["callID"],"name":part["tool"],"input":part["state"]["input"]})
            }
            _ => {
                out.push(ro::event(out.len(), "meta", json!([]), millis, part));
                continue;
            }
        };
        out.push(ro::event(out.len(),role,json!([block]),millis,json!({"message_id":row.get::<_,String>(0)?,"part_id":row.get::<_,String>(2)?,"part":part})));
        if part["type"] == "tool" {
            if let Some(output) = part["state"]
                .get("output")
                .or_else(|| part["state"].get("error"))
            {
                out.push(ro::event(
                    out.len(),
                    "user",
                    json!([{"type":"tool_result","tool_use_id":part["callID"],"content":output}]),
                    millis,
                    json!({"part":part}),
                ));
            }
        }
    }
    Ok(out)
}
fn read_summary(
    db: &Connection,
    root: &Path,
    id: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<SessionSummary> {
    let cols = ro::columns(db, "session")?;
    let opt = |n| ro::optional(&cols, n, "NULL");
    let sql = format!(
        "SELECT directory,title,time_created,time_updated,{},{} FROM session WHERE id=?1",
        opt("parent_id"),
        opt("time_archived")
    );
    let (cwd, title, created, updated, parent, archived) = db.query_row(&sql, [id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let events = read_events(db, id, cancel)?;
    let mut s = ro::summary(
        "zcode",
        id.into(),
        ro::locator("zcode", root, id),
        cwd,
        title,
        created / 1000,
        updated / 1000,
        &events,
        events.iter().map(|e| e.raw.to_string().len() as u64).sum(),
    );
    s.archived = archived.is_some();
    s.source = parent.map(|p| format!("parent:{p}"));
    Ok(s)
}
pub fn scan(
    root: &Path,
    cancel: Option<&AtomicBool>,
    callback: &mut dyn FnMut(usize, usize, &str, AppResult<SessionSummary>),
) -> AppResult<()> {
    ensure_not_cancelled(cancel)?;
    let db = ro::open_db(root, &database_path(root))?;
    let mut stmt = db.prepare("SELECT id FROM session ORDER BY time_updated DESC,id")?;
    let mut rows = stmt.query([])?;
    let mut ids = vec![];
    while let Some(row) = rows.next()? {
        ensure_not_cancelled(cancel)?;
        ids.push(row.get::<_, String>(0)?);
    }
    ro::register("zcode", root);
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
    let (claimed, id) = ro::decode("zcode", locator)?;
    if claimed != root {
        return Err(AppError::Path("ZCode 来源不匹配".into()));
    }
    let db = ro::open_db(root, &database_path(root))?;
    let s = read_summary(&db, root, &id, cancel)?;
    ro::register("zcode", root);
    Ok(Some(s))
}
pub fn events(locator: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let (root, id) = ro::registered("zcode", locator)?;
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
    let (root, id) = ro::registered("zcode", locator)?;
    Ok(ro::meta(read_summary(
        &ro::open_db(&root, &database_path(&root))?,
        &root,
        &id,
        None,
    )?))
}
