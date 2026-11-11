use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    dir: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(bytes: impl AsRef<[u8]>) -> AppResult<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "agentvault-claude-preview-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir)?;
        let fixture = Self {
            path: dir.join("synthetic.jsonl"),
            dir,
        };
        fs::write(&fixture.path, bytes)?;
        Ok(fixture)
    }

    fn path(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// Frozen alpha.50 paging order: classify every parsed value before applying the
// event offset. Keep this separate from the optimized reader for differential tests.
fn legacy_preview_range(path: &str, offset: usize, limit: usize) -> AppResult<Vec<PreviewEvent>> {
    let f = File::open(PathBuf::from(path))?;
    let reader = BufReader::new(f);
    let mut out = Vec::with_capacity(limit.min(1024));
    let mut event_index = 0usize;
    let mut parsed_lines = 0usize;
    let mut first_parse_error = None;
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw = match serde_json::from_str::<Value>(&line) {
            Ok(raw) => raw,
            Err(error) => {
                first_parse_error.get_or_insert(error);
                continue;
            }
        };
        parsed_lines += 1;
        if let Some(event) = classify_preview(i, raw) {
            if event_index < offset {
                event_index += 1;
                continue;
            }
            out.push(event);
            event_index += 1;
            if out.len() >= limit {
                break;
            }
        }
    }
    if parsed_lines == 0 {
        if let Some(error) = first_parse_error {
            return Err(error.into());
        }
    }
    Ok(out)
}

fn assert_same_result(path: &str, offset: usize, limit: usize) {
    let old = legacy_preview_range(path, offset, limit);
    let new = preview_range(path, offset, limit);
    match (old, new) {
        (Ok(old), Ok(new)) => assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(new).unwrap(),
            "offset={offset}, limit={limit}"
        ),
        (Err(AppError::Io(old)), Err(AppError::Io(new))) => {
            assert_eq!(old.kind(), new.kind(), "offset={offset}, limit={limit}");
        }
        (Err(AppError::Serde(old)), Err(AppError::Serde(new))) => {
            assert_eq!(old.classify(), new.classify());
            assert_eq!((old.line(), old.column()), (new.line(), new.column()));
        }
        (old, new) => {
            panic!("different results at offset={offset}, limit={limit}: {old:?} / {new:?}")
        }
    }
}

fn mixed_transcript() -> String {
    [
        "",
        " \t ",
        "{broken json",
        r#"{"type":"custom-title","customTitle":"标题"}"#,
        r#"{"type":"summary","summary":"摘要"}"#,
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"meta"}}"#,
        r#"{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":{}}}"#,
        "null",
        "42",
        "true",
        r#""plain JSON string""#,
        "[]",
        "{}",
        r#"{"type":23,"timestamp":false,"message":null}"#,
        r#"{"type":"user","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"中文 🐈 prompt"}}"#,
        "[invalid]",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"answer"},{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"result"}]}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"think"},{"type":"redacted_thinking","data":"opaque"}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"last answer"}]}}"#,
    ]
    .join("\r\n")
}

#[test]
fn mixed_transcript_pages_equal_full_read_and_keep_physical_indices() -> AppResult<()> {
    let fixture = Fixture::new(format!("{}\r\n", mixed_transcript()).repeat(17))?;
    let expected = legacy_preview_range(fixture.path(), 0, usize::MAX)?;
    assert!(expected.len() > 200);
    assert_eq!(expected[0].index, 3);
    assert_eq!(expected[0].role, "meta");
    assert_eq!(expected[3].role, "other");
    assert_eq!(expected[11].index, 14);
    assert_eq!(expected[11].role, "user");
    for page_size in [1, 2, 3, 7, 200] {
        let mut joined = Vec::new();
        loop {
            let page = preview_range(fixture.path(), joined.len(), page_size)?;
            let reached_end = page.len() < page_size;
            joined.extend(page);
            if reached_end {
                break;
            }
        }
        assert_eq!(
            serde_json::to_value(&joined)?,
            serde_json::to_value(&expected)?,
            "page_size={page_size}"
        );
    }
    Ok(())
}

#[test]
fn every_offset_and_extreme_limit_match_legacy_paging() -> AppResult<()> {
    // No final newline: the final valid event must still be returned.
    let fixture = Fixture::new(mixed_transcript())?;
    let count = legacy_preview_range(fixture.path(), 0, usize::MAX)?.len();
    for offset in (0..=count + 1).chain([usize::MAX]) {
        for limit in [0, 1, 2, 7, 200, usize::MAX] {
            assert_same_result(fixture.path(), offset, limit);
        }
    }
    // Preserve the existing zero-limit contract rather than changing it in a
    // performance patch: a nonempty remaining range yields its first event.
    assert_eq!(preview_range(fixture.path(), 0, 0)?.len(), 1);
    assert!(preview_range(fixture.path(), count, 0)?.is_empty());
    Ok(())
}

#[test]
fn malformed_empty_and_valid_skipped_prefix_keep_error_semantics() -> AppResult<()> {
    for bytes in ["", " \n\t\r\n", "broken\n{also broken\n", "broken\nnull\n"] {
        let fixture = Fixture::new(bytes)?;
        for offset in [0, 1, 99, usize::MAX] {
            for limit in [0, 1, usize::MAX] {
                assert_same_result(fixture.path(), offset, limit);
            }
        }
        if bytes == "broken\n{also broken\n" {
            assert!(matches!(
                preview_range(fixture.path(), usize::MAX, 1),
                Err(AppError::Serde(_))
            ));
        } else if bytes == "broken\nnull\n" {
            assert!(preview_range(fixture.path(), 1, 1)?.is_empty());
        }
    }
    Ok(())
}

#[test]
fn invalid_utf8_before_or_after_page_boundary_keeps_read_behavior() -> AppResult<()> {
    for bytes in [
        b"\xff\nnull\n".as_slice(),
        b"null\n\xff\nnull\n".as_slice(),
        b"null\nnull\n\xff\n".as_slice(),
    ] {
        let fixture = Fixture::new(bytes)?;
        for offset in [0, 1, 2, usize::MAX] {
            for limit in [0, 1, 2, usize::MAX] {
                assert_same_result(fixture.path(), offset, limit);
            }
        }
    }
    let fixture = Fixture::new(b"null\n\xff\n")?;
    assert_eq!(preview_range(fixture.path(), 0, 1)?.len(), 1);
    assert!(matches!(
        preview_range(fixture.path(), 1, 1),
        Err(AppError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData
    ));
    Ok(())
}

#[test]
fn missing_source_preserves_io_error_even_for_empty_requested_range() -> AppResult<()> {
    let fixture = Fixture::new("")?;
    fs::remove_file(&fixture.path)?;
    for offset in [0, usize::MAX] {
        for limit in [0, 1, usize::MAX] {
            assert_same_result(fixture.path(), offset, limit);
        }
    }
    assert!(matches!(
        preview_range(fixture.path(), 0, 0),
        Err(AppError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound
    ));
    Ok(())
}

#[test]
fn reread_observes_append_truncate_and_replaced_file() -> AppResult<()> {
    let fixture = Fixture::new("null\n")?;
    assert!(preview_range(fixture.path(), 1, 1)?.is_empty());
    {
        let mut out = fs::OpenOptions::new().append(true).open(&fixture.path)?;
        writeln!(out, "{{\"type\":\"summary\",\"summary\":\"appended\"}}")?;
    }
    assert_same_result(fixture.path(), 1, 1);
    assert_eq!(
        preview_range(fixture.path(), 1, 1)?[0].text_summary,
        "appended"
    );

    fs::write(&fixture.path, b"true\n")?;
    assert_same_result(fixture.path(), 1, 1);
    assert!(preview_range(fixture.path(), 1, 1)?.is_empty());
    assert_eq!(
        preview_range(fixture.path(), 0, 1)?[0].raw,
        Value::Bool(true)
    );

    let replacement = fixture.dir.join("replacement.jsonl");
    fs::write(&replacement, b"false\nnull\n")?;
    fs::remove_file(&fixture.path)?;
    fs::rename(replacement, &fixture.path)?;
    for offset in [0, 1, 2, usize::MAX] {
        assert_same_result(fixture.path(), offset, 2);
    }
    let events = preview_range(fixture.path(), 0, 2)?;
    assert_eq!(events[0].raw, Value::Bool(false));
    assert_eq!(events[1].raw, Value::Null);
    Ok(())
}
