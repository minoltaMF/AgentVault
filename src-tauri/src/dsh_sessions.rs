//! Read-only DSH v3/v4 transcript projection. Rules adapted from agent-sessions
//! ab439f13211e56b809f4a917d5c38e80d2bb5258 (MIT) and official DSH v4 catalog
//! 00102833dfaee1da9f48a3a8eae9d34005a75218 (MIT). See fixtures/dsh/README.md.
use crate::{
    error::{ensure_not_cancelled, AppError, AppResult},
    models::{PreviewEvent, SessionMetaBrief, SessionSummary},
    readonly_source as ro,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};
static ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
const MAX_BYTES: u64 = 128 * 1024 * 1024;
pub fn generation(path: &Path) -> Option<u64> {
    let n = path
        .file_name()?
        .to_str()?
        .strip_suffix(".zstd")
        .unwrap_or(path.file_name()?.to_str()?);
    if n == "session.jsonl" {
        return Some(0);
    }
    let v = n.strip_prefix("session.v")?.strip_suffix(".jsonl")?;
    let parsed = v.parse::<u64>().ok()?;
    if parsed > 0 && parsed.to_string() == v {
        Some(parsed)
    } else {
        None
    }
}
pub fn is_main_transcript(root: &Path, path: &Path) -> bool {
    generation(path).is_some()
        && path.strip_prefix(root.join("sessions")).is_ok_and(|p| {
            p.components().count() == 3
                && p.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        })
}
fn selected(root: &Path, path: &Path) -> AppResult<bool> {
    if !is_main_transcript(root, path) {
        return Err(AppError::Path("不是 DSH sessions 会话文件".into()));
    }
    ro::validate_file(root, path)?;
    let current = generation(path).unwrap();
    let mut same = 0;
    for entry in fs::read_dir(path.parent().unwrap())? {
        let p = entry?.path();
        if let Some(v) = generation(&p) {
            if v > current {
                return Ok(false);
            }
            if v == current {
                same += 1;
            }
        }
    }
    if same > 1 {
        return Err(AppError::Other(
            "同一 DSH 版本有多个物理文件，无法确定当前来源".into(),
        ));
    }
    Ok(true)
}
fn read(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<(Value, Vec<PreviewEvent>, String, i64, Option<String>)> {
    ensure_not_cancelled(cancel)?;
    if !selected(root, path)? {
        return Err(AppError::Other("DSH 会话已有更新格式，请重新扫描".into()));
    }
    let file = File::open(path)?;
    let input: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zstd") {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(input.take(MAX_BYTES + 1));
    let mut line = String::new();
    let mut used = 0;
    reader.read_line(&mut line)?;
    used += line.len() as u64;
    let header: Value = serde_json::from_str(&line)?;
    let version = header["version"].as_u64().unwrap_or(u64::MAX);
    if !matches!(version, 3 | 4) || generation(path) != Some(version) {
        return Err(AppError::Other(format!(
            "DSH 格式 v{version} 暂不支持；当前支持 v3/v4"
        )));
    }
    if header["type"] != "session"
        || header["id"].as_str().is_none_or(str::is_empty)
        || header["createdAt"].as_i64().is_none()
    {
        return Err(AppError::Other("DSH header 无效".into()));
    }
    let mut out = vec![];
    let mut seq = 0;
    let mut title = String::new();
    let mut updated = header["createdAt"].as_i64().unwrap();
    let mut model = None;
    let known: HashSet<&str> = include_str!("../tests/fixtures/dsh/known-events.txt")
        .lines()
        .collect();
    loop {
        ensure_not_cancelled(cancel)?;
        line.clear();
        let len = reader.read_line(&mut line)?;
        if len == 0 {
            break;
        }
        used += len as u64;
        if used > MAX_BYTES {
            return Err(AppError::Other("DSH 解压正文超过 128 MiB 读取上限".into()));
        }
        if line.trim().is_empty() {
            continue;
        }
        if !line.ends_with('\n') {
            return Err(AppError::Other("DSH 日志尾部尚未写完，请稍后刷新".into()));
        }
        let row: Value = serde_json::from_str(&line)?;
        if row["seq"].as_u64() != Some(seq) {
            return Err(AppError::Other(format!("DSH 日志序号不连续：期待 {seq}")));
        }
        seq += 1;
        let kind = row["type"]
            .as_str()
            .ok_or_else(|| AppError::Other("DSH 事件缺少 type".into()))?;
        let millis = row["time"]
            .as_i64()
            .ok_or_else(|| AppError::Other("DSH 事件缺少 time".into()))?;
        updated = updated.max(millis);
        if !known.contains(kind) && row["ignorable"] != true {
            return Err(AppError::Other(format!("DSH 未知必要事件：{kind}")));
        }
        let data = &row["data"];
        if kind == "session/title" {
            title = data["title"].as_str().unwrap_or("").into();
        }
        if kind == "request/header" {
            model = data
                .pointer("/header/config/model")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        let (role, message) = match kind {
            "user/message"
                if data.pointer("/source/kind").and_then(Value::as_str) == Some("user") =>
            {
                ("user", data)
            }
            "assistant/message" => ("assistant", &data["message"]),
            "tool/result" => ("user", &data["message"]),
            // Diagnostic attempts and compaction/injected context are not human dialogue.
            _ => {
                out.push(ro::event(
                    out.len(),
                    "meta",
                    json!([]),
                    millis,
                    json!({"type":kind,"seq":row["seq"]}),
                ));
                continue;
            }
        };
        let blocks = message["content"]
            .as_array()
            .ok_or_else(|| AppError::Other(format!("DSH {kind} 缺少消息 content")))?;
        let mut content = vec![];
        for block in blocks {
            match block["type"].as_str(){
                Some("text")=>content.push(json!({"type":"text","text":block["text"]})),
                Some("reasoning")=>content.push(json!({"type":"thinking","thinking":block["text"]})),
                Some("tool-call")=>content.push(json!({"type":"tool_use","id":block["id"],"name":block["name"],"input":block["arguments"]})),
                Some("tool-result")=>content.push(json!({"type":"tool_result","tool_use_id":block["toolCallId"],"content":ro::text(&block["content"])})),
                Some("image"|"file")=>content.push(json!({"type":"text","text":"[附件，未读取内容]"})),
                Some("tool-addition"|"tool-removal")=>content.push(json!({"type":"dsh_tool_metadata","native":block})),
                _=>return Err(AppError::Other("DSH 消息包含未知内容块".into()))
            }
        }
        // v4 moved tool identity to the message envelope. Its content is ordinary
        // text/image blocks, not the v3 nested tool-result block; keep it out of
        // human dialogue and preserve call correlation for the preview.
        if version == 4 && kind == "tool/result" {
            let call_id = message["toolCallId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| AppError::Other("DSH v4 tool/result 缺少 toolCallId".into()))?;
            content = vec![json!({
                "type":"tool_result",
                "tool_use_id":call_id,
                "is_error":message["isError"].as_bool().unwrap_or(false),
                "content":content
            })];
        }
        let role = if !content.is_empty()
            && content
                .iter()
                .all(|block| block["type"] == "dsh_tool_metadata")
        {
            "meta"
        } else {
            role
        };
        out.push(ro::event(
            out.len(),
            role,
            Value::Array(content),
            millis,
            json!({"type":kind,"seq":row["seq"],"message_id":message["id"]}),
        ));
    }
    Ok((header, out, title, updated, model))
}
pub fn parse_session(
    root: &Path,
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<SessionSummary>> {
    if !selected(root, path)? {
        return Ok(None);
    }
    let (h, events, title, updated, model) = read(root, path, cancel)?;
    let mut s = ro::summary(
        "dsh",
        h["id"].as_str().unwrap().into(),
        path.to_string_lossy().into_owned(),
        h["cwd"].as_str().unwrap_or("").into(),
        title,
        h["createdAt"].as_i64().unwrap() / 1000,
        updated / 1000,
        &events,
        fs::metadata(path)?.len(),
    );
    s.model = model;
    s.source = Some(
        if h["origin"] == "subagent" {
            "subagent"
        } else {
            "cli"
        }
        .into(),
    );
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.into());
    Ok(Some(s))
}
fn root_for(path: &Path) -> AppResult<PathBuf> {
    ROOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|r| is_main_transcript(r, path))
        .cloned()
        .ok_or_else(|| AppError::Path("请先扫描 DSH 来源".into()))
}
pub fn events(path: &str, cancel: Option<&AtomicBool>) -> AppResult<Vec<PreviewEvent>> {
    let path = Path::new(path);
    Ok(read(&root_for(path)?, path, cancel)?.1)
}
pub fn preview_range(path: &str, offset: usize, limit: usize) -> AppResult<Vec<PreviewEvent>> {
    Ok(events(path, None)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}
pub fn preview_meta(path: &str) -> AppResult<SessionMetaBrief> {
    let p = Path::new(path);
    Ok(ro::meta(
        parse_session(&root_for(p)?, p, None)?
            .ok_or_else(|| AppError::Other("DSH 版本已变化".into()))?,
    ))
}
