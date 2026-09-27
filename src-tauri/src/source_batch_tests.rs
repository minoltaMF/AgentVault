//! Synthetic sources only: no native user history is read or modified.
use crate::{error::AppResult, models::SessionSummary};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "agentvault-batch10-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn workbench_four_sources_search_and_preview_share_exact_positions() -> AppResult<()> {
    let f = Fixture::new();
    let dsh = f.0.join("dsh");
    let hermes = f.0.join("hermes");
    let zcode = f.0.join("zcode");
    let opencode = f.0.join("opencode");
    let dpath = dsh.join("sessions/project/same/session.v4.jsonl");
    fs::create_dir_all(dpath.parent().unwrap())?;
    fs::write(
        &dpath,
        include_str!("../tests/fixtures/dsh/sample-v4.jsonl"),
    )?;
    for (root, path, sql) in [
        (
            &hermes,
            crate::hermes_sessions::database_path(&hermes),
            include_str!("../tests/fixtures/hermes/sample.sql"),
        ),
        (
            &zcode,
            crate::zcode_sessions::database_path(&zcode),
            include_str!("../tests/fixtures/zcode/sample.sql"),
        ),
        (
            &opencode,
            crate::opencode_sessions::database_path(&opencode),
            include_str!("../tests/fixtures/zcode/sample.sql"),
        ),
    ] {
        fs::create_dir_all(path.parent().unwrap())?;
        let db = rusqlite::Connection::open(path)?;
        db.execute_batch(sql)?;
        if root == &opencode {
            db.execute_batch("ALTER TABLE session ADD COLUMN project_id TEXT DEFAULT 'project'; ALTER TABLE session ADD COLUMN version TEXT DEFAULT 'fixture';")?;
        }
    }
    let mut all = vec![];
    for provider in ["dsh", "hermes", "opencode", "zcode"] {
        let started = crate::workbench_scan::start_workbench_scan(
            provider.into(),
            f.0.to_string_lossy().into_owned(),
            String::new(),
            None,
            None,
            None,
            None,
            Some(dsh.to_string_lossy().into_owned()),
            Some(hermes.to_string_lossy().into_owned()),
            Some(zcode.to_string_lossy().into_owned()),
            Some(opencode.to_string_lossy().into_owned()),
            None,
            None,
            None,
            None,
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let status = crate::workbench_scan::workbench_scan_status(started.job_id)?;
            if status.state != "running" {
                assert_eq!(status.state, "completed", "{:?}", status.error);
                assert_eq!(status.failed_files, 0);
                all.extend(status.results);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert_eq!(all.len(), 4);
    let scopes = all
        .iter()
        .map(|s| crate::content_search::ContentSearchScope {
            provider: s.provider.clone(),
            rollout_paths: vec![s.rollout_path.clone()],
        })
        .collect();
    let started = crate::content_search::start_workbench_content_search(
        crate::models::ProviderDirs {
            codex_dir: f.0.to_string_lossy().into_owned(),
            dsh_dir: Some(dsh.to_string_lossy().into_owned()),
            hermes_dir: Some(hermes.to_string_lossy().into_owned()),
            zcode_dir: Some(zcode.to_string_lossy().into_owned()),
            opencode_dir: Some(opencode.to_string_lossy().into_owned()),
            ..Default::default()
        },
        "needle".into(),
        scopes,
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let status = crate::content_search::content_search_status(started.job_id)?;
        if status.state != "running" {
            assert_eq!(status.state, "completed", "{:?}", status.error);
            assert_eq!(status.failed_files, 0, "{:?}", status.failures);
            assert_eq!(status.results.len(), 4);
            for result in status.results {
                if result.session.provider == "dsh" {
                    assert_eq!(
                        result.matches.len(),
                        2,
                        "tool output must not match dialogue"
                    );
                }
                for hit in result.matches {
                    let events = crate::rollout::preview_session_range(
                        Some(result.session.provider.clone()),
                        result.session.rollout_path.clone(),
                        hit.event_offset,
                        1,
                    )?;
                    assert_eq!(events[0].index, hit.event_index);
                    assert!(crate::rollout::preview_event_text(&events[0]).contains("needle"));
                }
            }
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
}

#[test]
fn sqlite_sources_preserve_native_bytes_order_and_source_identity() -> AppResult<()> {
    for provider in ["hermes", "zcode"] {
        let f = Fixture::new();
        let path = if provider == "hermes" {
            crate::hermes_sessions::database_path(&f.0)
        } else {
            crate::zcode_sessions::database_path(&f.0)
        };
        fs::create_dir_all(path.parent().unwrap())?;
        let db = rusqlite::Connection::open(&path)?;
        db.execute_batch(if provider == "hermes" {
            include_str!("../tests/fixtures/hermes/sample.sql")
        } else {
            include_str!("../tests/fixtures/zcode/sample.sql")
        })?;
        drop(db);
        let before = fs::read(&path)?;
        let mut sessions = vec![];
        let mut collect = |done, total, _: &str, s: AppResult<SessionSummary>| {
            assert!(done <= total);
            sessions.push(s.unwrap());
        };
        if provider == "hermes" {
            crate::hermes_sessions::scan(&f.0, None, &mut collect)?;
        } else {
            crate::zcode_sessions::scan(&f.0, None, &mut collect)?;
        }
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.provider, provider);
        assert!(s.first_user_message.contains("question"));
        let events = if provider == "hermes" {
            crate::hermes_sessions::events(&s.rollout_path, None)?
        } else {
            crate::zcode_sessions::events(&s.rollout_path, None)?
        };
        assert_eq!(events[0].role, "user");
        assert_eq!(events[1].role, "assistant");
        assert!(crate::readonly_source::registered(
            if provider == "hermes" {
                "zcode"
            } else {
                "hermes"
            },
            &s.rollout_path
        )
        .is_err());
        let cancelled = AtomicBool::new(true);
        let result = if provider == "hermes" {
            crate::hermes_sessions::events(&s.rollout_path, Some(&cancelled))
        } else {
            crate::zcode_sessions::events(&s.rollout_path, Some(&cancelled))
        };
        assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
        assert_eq!(before, fs::read(&path)?);
    }
    Ok(())
}
#[test]
fn dsh_compressed_generation_and_failure_contract() -> AppResult<()> {
    let f = Fixture::new();
    let directory = f.0.join("sessions/project/same");
    fs::create_dir_all(&directory)?;
    let sample = include_str!("../tests/fixtures/dsh/sample-v4.jsonl");
    let old = directory.join("session.v3.jsonl");
    fs::write(&old, sample.replace("\"version\":4", "\"version\":3"))?;
    let path = directory.join("session.v4.jsonl.zstd");
    let split = sample.find('\n').unwrap() + 1;
    let mut compressed = zstd::stream::encode_all(&sample.as_bytes()[..split], 1)?;
    compressed.extend(zstd::stream::encode_all(&sample.as_bytes()[split..], 1)?);
    fs::write(&path, &compressed)?;
    assert!(crate::dsh_sessions::parse_session(&f.0, &old, None)?.is_none());
    let s = crate::dsh_sessions::parse_session(&f.0, &path, None)?.unwrap();
    assert!(s.first_user_message.contains("question"));
    let events = crate::dsh_sessions::events(&s.rollout_path, None)?;
    let tool = events
        .iter()
        .find(|event| event.role == "tool_result")
        .unwrap();
    assert_eq!(
        tool.raw["message"]["content"][0]["tool_use_id"],
        "call-fixture"
    );
    assert_eq!(tool.raw["message"]["content"][0]["is_error"], true);
    assert!(crate::rollout::preview_event_text(tool).contains("tool output"));
    let metadata = events
        .iter()
        .find(|event| event.raw["native"]["message_id"] == "tools")
        .unwrap();
    assert_eq!(metadata.role, "meta");
    assert_eq!(
        metadata.raw["message"]["content"][0]["native"]["type"],
        "tool-addition"
    );
    assert_eq!(
        metadata.raw["message"]["content"][1]["native"]["type"],
        "tool-removal"
    );
    assert!(crate::rollout::preview_event_text(metadata).is_empty());
    assert_eq!(
        events
            .iter()
            .filter(|e| crate::rollout::preview_event_is_conversation(e))
            .count(),
        2
    );
    assert!(!events
        .iter()
        .any(|e| crate::rollout::preview_event_text(e).contains("failed attempt")));
    assert_eq!(compressed, fs::read(&path)?);
    fs::write(
        &path,
        zstd::stream::encode_all(sample.replace("\"seq\":2", "\"seq\":8").as_bytes(), 1)?,
    )?;
    assert!(crate::dsh_sessions::parse_session(&f.0, &path, None).is_err());
    fs::write(&path, &compressed[..compressed.len() - 3])?;
    assert!(crate::dsh_sessions::parse_session(&f.0, &path, None).is_err());
    fs::write(directory.join("session.v5.jsonl"), "{}\n")?;
    assert!(crate::dsh_sessions::parse_session(&f.0, &old, None)?.is_none());
    assert!(
        crate::dsh_sessions::parse_session(&f.0, &directory.join("session.v5.jsonl"), None)
            .is_err()
    );
    Ok(())
}

#[test]
fn batch11_workbench_scans_searches_and_locates_readonly_content() -> AppResult<()> {
    let f = Fixture::new();
    let qwen = f.0.join("qwen");
    let cline = f.0.join("cline");
    let copilot = f.0.join("copilot");
    let antigravity = f.0.join("antigravity");
    let files = [
        (
            qwen.join("projects/demo/chats/019f0000-0000-7000-8000-000000000001.jsonl"),
            include_str!("../tests/fixtures/qwen/019f0000-0000-7000-8000-000000000001.jsonl"),
        ),
        (
            cline.join("sessions/cline-cli-tool/cline-cli-tool.json"),
            include_str!("../tests/fixtures/cline/cli_tool/cline-cli-tool.json"),
        ),
        (
            cline.join("sessions/cline-cli-tool/cline-cli-tool.messages.json"),
            include_str!("../tests/fixtures/cline/cli_tool/cline-cli-tool.messages.json"),
        ),
        (
            copilot.join("session-state/copilot_stage0_small/events.jsonl"),
            include_str!("../tests/fixtures/copilot/copilot_stage0_small.jsonl"),
        ),
        (
            antigravity.join("antigravity-cli/brain/demo/.system_generated/logs/transcript.jsonl"),
            include_str!("../tests/fixtures/antigravity/cli_small.jsonl"),
        ),
        (
            antigravity.join("antigravity/brain/demo/task.md"),
            include_str!("../tests/fixtures/antigravity/small.md"),
        ),
    ];
    for (path, content) in &files {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, content)?;
    }
    // One broken source must remain a visible per-file failure alongside usable sessions.
    let bad = qwen.join("projects/demo/chats/019f0000-0000-7000-8000-000000000002.jsonl");
    fs::write(&bad, "{broken\n")?;
    let dirs = crate::models::ProviderDirs {
        codex_dir: f.0.to_string_lossy().into_owned(),
        qwen_dir: Some(qwen.to_string_lossy().into_owned()),
        cline_dir: Some(cline.to_string_lossy().into_owned()),
        copilot_dir: Some(copilot.to_string_lossy().into_owned()),
        antigravity_dir: Some(antigravity.to_string_lossy().into_owned()),
        ..Default::default()
    };
    for (provider, query, count) in [
        ("qwen", "visible question", 1),
        ("cline", "Inspect fixture.txt", 1),
        ("copilot", "List the files", 1),
        ("antigravity", "List the files", 2),
    ] {
        let job = crate::workbench_scan::start_workbench_scan(
            provider.into(),
            dirs.codex_dir.clone(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            dirs.qwen_dir.clone(),
            dirs.cline_dir.clone(),
            dirs.copilot_dir.clone(),
            dirs.antigravity_dir.clone(),
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let sessions = loop {
            let status = crate::workbench_scan::workbench_scan_status(job.job_id)?;
            if status.state != "running" {
                assert_eq!(status.state, "completed", "{:?}", status.error);
                assert_eq!(status.failed_files, if provider == "qwen" { 1 } else { 0 });
                assert_eq!(status.results.len(), count);
                break status.results;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(sessions.iter().all(|s| s.resume_command.is_empty()));
        if provider == "antigravity" {
            assert!(sessions
                .iter()
                .any(|s| s.source.as_deref() == Some("artifact-incomplete")
                    && s.title.contains("非完整对话")));
        }
        let job = crate::content_search::start_workbench_content_search(
            dirs.clone(),
            query.into(),
            vec![crate::content_search::ContentSearchScope {
                provider: provider.into(),
                rollout_paths: sessions.iter().map(|s| s.rollout_path.clone()).collect(),
            }],
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let status = crate::content_search::content_search_status(job.job_id)?;
            if status.state != "running" {
                assert_eq!(status.state, "completed", "{:?}", status.error);
                assert_eq!(status.failed_files, 0, "{:?}", status.failures);
                assert!(!status.results.is_empty(), "{provider}");
                for result in status.results {
                    for hit in result.matches {
                        let events = crate::rollout::preview_session_range(
                            Some(provider.into()),
                            result.session.rollout_path.clone(),
                            hit.event_offset,
                            1,
                        )?;
                        assert_eq!(events[0].index, hit.event_index);
                        assert!(crate::rollout::preview_event_text(&events[0]).contains(query));
                    }
                }
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    for (path, content) in files {
        assert_eq!(fs::read_to_string(path)?, content);
    }
    Ok(())
}
