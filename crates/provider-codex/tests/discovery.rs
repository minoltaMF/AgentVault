use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use provider_codex::CodexProvider;
use provider_sdk::{
    DetectionContext, ProviderCapabilities, ProviderContext, SessionProvider, SessionRootKind,
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
fn discovers_active_and_archived_rollouts_as_one_raw_inventory() {
    let home = temp_dir("codex-discovery");
    let active_id = "019d0000-1111-7000-8000-000000000001";
    let archived_id = "019d0000-1111-7000-8000-000000000002";
    let active = home
        .join("sessions/2026/09/05")
        .join(format!("rollout-2026-09-05T10-00-00-{active_id}.jsonl"));
    let archived = home
        .join("archived_sessions")
        .join(format!("rollout-2026-09-04T10-00-00-{archived_id}.jsonl"));
    write(&active, "active\n");
    write(&archived, "archived\n");
    write(
        &home.join("sessions/2026/09/05/not-a-rollout.jsonl"),
        "ignored\n",
    );
    let active_before = fs::read(&active).expect("read active");
    let archived_before = fs::read(&archived).expect("read archived");

    let provider = CodexProvider;
    let roots = provider
        .roots(&ProviderContext::new(&home))
        .expect("list roots");
    let page = provider
        .discover(&ProviderContext::new(&home), None)
        .expect("discover rollouts");

    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].kind, SessionRootKind::Active);
    assert_eq!(roots[1].kind, SessionRootKind::Archived);
    assert_eq!(page.sessions.len(), 2);
    assert_eq!(
        page.sessions
            .iter()
            .find(|session| session.session_root.kind == SessionRootKind::Active)
            .and_then(|session| session.native_session_id.as_deref()),
        Some(active_id)
    );
    assert_eq!(
        page.sessions
            .iter()
            .find(|session| session.session_root.kind == SessionRootKind::Archived)
            .and_then(|session| session.native_session_id.as_deref()),
        Some(archived_id)
    );
    assert_eq!(fs::read(&active).expect("reread active"), active_before);
    assert_eq!(
        fs::read(&archived).expect("reread archived"),
        archived_before
    );

    fs::remove_dir_all(home).ok();
}

#[test]
fn state_database_alone_is_detection_evidence_but_not_a_native_session() {
    let home = temp_dir("codex-detection");
    write(&home.join("state_5.sqlite"), "fixture\n");
    let provider = CodexProvider;

    let detection = provider
        .detect(&DetectionContext::new(&home))
        .expect("detect Codex home");
    let page = provider
        .discover(&ProviderContext::new(&home), None)
        .expect("discover empty rollout roots");

    assert!(detection.detected);
    assert_eq!(detection.evidence, vec![home.join("state_5.sqlite")]);
    assert!(page.sessions.is_empty());
    fs::remove_dir_all(home).ok();
}

#[test]
fn cancellation_stops_discovery() {
    let home = temp_dir("codex-cancel");
    write(
        &home.join("sessions/2026/09/05/rollout-any.jsonl"),
        "fixture\n",
    );
    let cancel = AtomicBool::new(true);

    let error = CodexProvider
        .discover(
            &ProviderContext::new(&home).with_cancellation(&cancel),
            None,
        )
        .expect_err("cancelled discovery must fail");

    assert!(matches!(error, provider_sdk::DiscoveryError::Cancelled));
    fs::remove_dir_all(home).ok();
}

#[test]
fn descriptor_claims_only_migrated_discovery_capability() {
    let descriptor = CodexProvider.descriptor();

    assert_eq!(descriptor.id, "codex");
    assert!(descriptor.supports(ProviderCapabilities::DISCOVER));
    assert!(!descriptor.supports(ProviderCapabilities::PARSE));
    assert!(!descriptor.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}
