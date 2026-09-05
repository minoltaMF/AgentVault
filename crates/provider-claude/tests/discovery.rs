use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};

use provider_claude::ClaudeProvider;
use provider_sdk::{
    DetectionContext, NativeSessionKind, ProviderCapabilities, ProviderContext, SessionProvider,
    SessionRootKind,
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("agentvault-{label}-{}-{id}", std::process::id()))
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(path, contents).expect("write fixture");
}

#[test]
fn discovers_primary_and_subagent_transcripts_without_mutating_them() {
    let home = temp_dir("claude-discovery");
    let primary = home.join("projects/project-a/session-file.jsonl");
    let subagent = home.join("projects/project-a/session-file/subagents/agent-worker.jsonl");
    write(
        &primary,
        "not json\n{\"sessionId\":\"session-from-transcript\"}\n",
    );
    write(&subagent, "{\"sessionId\":\"parent-session\"}\n");
    write(&home.join("projects/project-a/ignore.txt"), "ignored\n");
    let primary_before = fs::read(&primary).expect("read primary");
    let subagent_before = fs::read(&subagent).expect("read subagent");

    let provider = ClaudeProvider;
    let detection = provider
        .detect(&DetectionContext::new(&home))
        .expect("detect Claude home");
    let roots = provider
        .roots(&ProviderContext::new(&home))
        .expect("list roots");
    let page = provider
        .discover(&ProviderContext::new(&home), None)
        .expect("discover transcripts");

    assert!(detection.detected);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].kind, SessionRootKind::Active);
    assert_eq!(page.sessions.len(), 2);
    let main = page
        .sessions
        .iter()
        .find(|session| session.kind == NativeSessionKind::Primary)
        .expect("primary transcript");
    assert_eq!(
        main.native_session_id.as_deref(),
        Some("session-from-transcript")
    );
    assert!(main.parent_native_session_id.is_none());
    let child = page
        .sessions
        .iter()
        .find(|session| session.kind == NativeSessionKind::Subagent)
        .expect("subagent transcript");
    assert_eq!(child.native_session_id.as_deref(), Some("agent-worker"));
    assert_eq!(
        child.parent_native_session_id.as_deref(),
        Some("parent-session")
    );
    assert_eq!(fs::read(&primary).expect("reread primary"), primary_before);
    assert_eq!(
        fs::read(&subagent).expect("reread subagent"),
        subagent_before
    );

    fs::remove_dir_all(home).ok();
}

#[test]
fn missing_home_is_not_detected_and_discovers_no_sessions() {
    let home = temp_dir("claude-missing");
    let provider = ClaudeProvider;

    let detection = provider
        .detect(&DetectionContext::new(&home))
        .expect("detect missing home");
    let page = provider
        .discover(&ProviderContext::new(&home), None)
        .expect("discover missing home");

    assert!(!detection.detected);
    assert!(detection.evidence.is_empty());
    assert!(page.sessions.is_empty());
}

#[test]
fn cancellation_stops_discovery_before_reading_transcripts() {
    let home = temp_dir("claude-cancel");
    write(
        &home.join("projects/project-a/session.jsonl"),
        "{\"sessionId\":\"session\"}\n",
    );
    let cancel = AtomicBool::new(true);
    let provider = ClaudeProvider;

    let error = provider
        .discover(
            &ProviderContext::new(&home).with_cancellation(&cancel),
            None,
        )
        .expect_err("cancelled discovery must fail");

    assert!(matches!(error, provider_sdk::DiscoveryError::Cancelled));
    fs::remove_dir_all(home).ok();
}

#[test]
fn descriptor_claims_migrated_discovery_and_subagent_lineage() {
    let descriptor = ClaudeProvider.descriptor();

    assert_eq!(descriptor.id, "claude");
    assert!(descriptor.supports(ProviderCapabilities::DISCOVER));
    assert!(descriptor.supports(ProviderCapabilities::SUBAGENT_LINEAGE));
    assert!(!descriptor.supports(ProviderCapabilities::PARSE));
    assert!(!descriptor.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}
