use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use provider_pi::{PiProvider, PiSessionError};
use provider_sdk::{
    ConsistencyClass, DetectionContext, NativeSessionKind, NativeSessionRef, ProviderCapabilities,
    ProviderContext, SessionProvider, SessionRoot, SessionRootKind,
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

fn discover_one(home: &Path) -> provider_sdk::NativeSessionRef {
    let page = PiProvider
        .discover(&ProviderContext::new(home), None)
        .expect("discover Pi sessions");
    assert_eq!(page.sessions.len(), 1);
    page.sessions.into_iter().next().expect("one session")
}

#[test]
fn discovers_nested_pi_sessions_from_header_without_mutating_them() {
    let home = temp_dir("pi-discovery");
    let path = home.join(
        "sessions/--workspace--/2026-09-05T10-00-00_019d0000-1111-7000-8000-000000000001.jsonl",
    );
    write(
        &path,
        "{\"type\":\"session\",\"version\":3,\"id\":\"pi-session-id\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\"}\n",
    );
    write(
        &home.join("sessions/--workspace--/ignored.txt"),
        "ignored\n",
    );
    let before = fs::read(&path).expect("read fixture");

    let detection = PiProvider
        .detect(&DetectionContext::new(&home))
        .expect("detect Pi home");
    let roots = PiProvider
        .roots(&ProviderContext::new(&home))
        .expect("list Pi roots");
    let native = discover_one(&home);

    assert!(detection.detected);
    assert_eq!(detection.evidence, vec![home.join("sessions")]);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].kind, SessionRootKind::Active);
    assert_eq!(native.native_session_id.as_deref(), Some("pi-session-id"));
    assert_eq!(native.source_format_version.as_deref(), Some("3"));
    assert_eq!(fs::read(&path).expect("reread fixture"), before);

    fs::remove_dir_all(home).ok();
}

#[test]
fn parses_v3_tree_without_discarding_inactive_branches_or_unknown_raw_events() {
    let home = temp_dir("pi-v3-branches");
    let path = home.join("sessions/--workspace--/session.jsonl");
    write(
        &path,
        concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"session-v3\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\",\"parentSession\":\"/sessions/original.jsonl\",\"extensionHeader\":true}\n",
            "{\"type\":\"message\",\"id\":\"root\",\"parentId\":null,\"timestamp\":\"2026-09-05T10:00:01Z\",\"message\":{\"role\":\"user\",\"content\":\"task\"}}\n",
            "{\"type\":\"custom\",\"id\":\"left\",\"parentId\":\"root\",\"timestamp\":\"2026-09-05T10:00:02Z\",\"customType\":\"extension.left\",\"data\":{\"keep\":true}}\n",
            "{\"type\":\"vendor_extension\",\"id\":\"right\",\"parentId\":\"root\",\"timestamp\":\"2026-09-05T10:00:03Z\",\"payload\":[1,2,3]}\n",
            "{\"type\":\"message\",\"id\":\"right-leaf\",\"parentId\":\"right\",\"timestamp\":\"2026-09-05T10:00:04Z\",\"message\":{\"role\":\"assistant\",\"content\":\"done\"}}\n"
        ),
    );
    let before = fs::read(&path).expect("read fixture");
    let native = discover_one(&home);

    let session = PiProvider
        .read_session(&ProviderContext::new(&home), &native)
        .expect("parse v3 session");

    assert_eq!(session.header.version, 3);
    assert_eq!(session.header.id, "session-v3");
    assert_eq!(
        session.header.parent_session.as_deref(),
        Some("/sessions/original.jsonl")
    );
    assert_eq!(session.header.raw["extensionHeader"], true);
    assert_eq!(session.entries.len(), 4);
    assert_eq!(
        session
            .graph
            .nodes()
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "left", "right", "right-leaf"]
    );
    assert_eq!(
        session
            .graph
            .active_path()
            .expect("active path")
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "right", "right-leaf"]
    );
    assert_eq!(session.entries[1].raw["data"]["keep"], true);
    assert_eq!(session.entries[2].entry_type, "vendor_extension");
    assert_eq!(session.entries[2].raw["payload"][2], 3);
    assert!(session.entries[2].raw_json.contains("\"payload\":[1,2,3]"));
    assert!(session.partial_tail.is_none());
    assert_eq!(fs::read(&path).expect("reread fixture"), before);

    fs::remove_dir_all(home).ok();
}

#[test]
fn supports_v1_linear_sessions_and_v2_tree_sessions() {
    let v1_home = temp_dir("pi-v1");
    write(
        &v1_home.join("sessions/project/v1.jsonl"),
        concat!(
            "{\"type\":\"session\",\"id\":\"session-v1\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\"}\n",
            "{\"type\":\"message\",\"timestamp\":\"2026-09-05T10:00:01Z\",\"message\":{\"role\":\"user\"}}\n",
            "{\"type\":\"message\",\"timestamp\":\"2026-09-05T10:00:02Z\",\"message\":{\"role\":\"assistant\"}}\n"
        ),
    );
    let v1_native = discover_one(&v1_home);
    let v1 = PiProvider
        .read_session(&ProviderContext::new(&v1_home), &v1_native)
        .expect("parse v1 session");
    assert_eq!(v1_native.source_format_version.as_deref(), Some("1"));
    assert_eq!(v1.header.version, 1);
    assert_eq!(v1.entries[0].node.id, "legacy:0");
    assert_eq!(v1.entries[1].node.id, "legacy:1");
    assert_eq!(v1.entries[1].node.parent_id.as_deref(), Some("legacy:0"));

    let v2_home = temp_dir("pi-v2");
    write(
        &v2_home.join("sessions/project/v2.jsonl"),
        concat!(
            "{\"type\":\"session\",\"version\":2,\"id\":\"session-v2\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\"}\n",
            "{\"type\":\"message\",\"id\":\"a\",\"parentId\":null,\"timestamp\":\"2026-09-05T10:00:01Z\"}\n",
            "{\"type\":\"hookMessage\",\"id\":\"b\",\"parentId\":\"a\",\"timestamp\":\"2026-09-05T10:00:02Z\",\"hookName\":\"legacy\"}\n"
        ),
    );
    let v2_native = discover_one(&v2_home);
    let v2 = PiProvider
        .read_session(&ProviderContext::new(&v2_home), &v2_native)
        .expect("parse v2 session");
    assert_eq!(v2.header.version, 2);
    assert_eq!(v2.entries[1].entry_type, "hookMessage");
    assert_eq!(v2.entries[1].raw["hookName"], "legacy");
    assert_eq!(v2.graph.active_node_id(), Some("b"));

    fs::remove_dir_all(v1_home).ok();
    fs::remove_dir_all(v2_home).ok();
}

#[test]
fn keeps_an_unterminated_partial_tail_out_of_the_branch_graph() {
    let home = temp_dir("pi-partial-tail");
    write(
        &home.join("sessions/project/live.jsonl"),
        concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"live\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\"}\n",
            "{\"type\":\"message\",\"id\":\"a\",\"parentId\":null,\"timestamp\":\"2026-09-05T10:00:01Z\"}\n",
            "{\"type\":\"message\",\"id\":\"unfinished\""
        ),
    );
    let native = discover_one(&home);

    let session = PiProvider
        .read_session(&ProviderContext::new(&home), &native)
        .expect("ignore live partial tail");

    assert_eq!(session.entries.len(), 1);
    let tail = session.partial_tail.expect("partial tail");
    assert_eq!(tail.line_number, 3);
    assert!(tail.raw.contains("unfinished"));

    fs::remove_dir_all(home).ok();
}

#[test]
fn refuses_to_read_a_reference_outside_the_configured_session_root() {
    let home = temp_dir("pi-path-guard");
    let outside = temp_dir("pi-path-guard-outside").join("outside.jsonl");
    write(&home.join("sessions/keep-root-present.jsonl"), "{}\n");
    write(
        &outside,
        "{\"type\":\"session\",\"version\":3,\"id\":\"outside\"}\n",
    );
    let before = fs::read(&outside).expect("read outside fixture");
    let native = NativeSessionRef::new(
        "pi",
        Some("outside".into()),
        &outside,
        SessionRoot::new(
            home.join("sessions"),
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    let error = PiProvider
        .read_session(&ProviderContext::new(&home), &native)
        .expect_err("path escape must be rejected");

    assert!(matches!(error, PiSessionError::UnsafePath { .. }));
    assert_eq!(fs::read(&outside).expect("reread outside fixture"), before);

    fs::remove_dir_all(home).ok();
    fs::remove_dir_all(outside.parent().expect("outside parent")).ok();
}

#[test]
fn rejects_unsupported_versions_and_honors_cancellation() {
    let home = temp_dir("pi-version-cancel");
    write(
        &home.join("sessions/project/v4.jsonl"),
        "{\"type\":\"session\",\"version\":4,\"id\":\"future\",\"timestamp\":\"2026-09-05T10:00:00Z\",\"cwd\":\"/workspace\"}\n",
    );
    let native = discover_one(&home);
    let error = PiProvider
        .read_session(&ProviderContext::new(&home), &native)
        .expect_err("future format must not be guessed");
    assert!(matches!(
        error,
        PiSessionError::UnsupportedVersion { version: 4 }
    ));

    let cancel = AtomicBool::new(true);
    let error = PiProvider
        .discover(
            &ProviderContext::new(&home).with_cancellation(&cancel),
            None,
        )
        .expect_err("cancelled discovery must fail");
    assert!(matches!(error, provider_sdk::DiscoveryError::Cancelled));

    fs::remove_dir_all(home).ok();
}

#[test]
fn descriptor_claims_only_implemented_read_capabilities() {
    let descriptor = PiProvider.descriptor();

    assert_eq!(descriptor.id, "pi");
    assert!(descriptor.supports(ProviderCapabilities::DISCOVER));
    assert!(descriptor.supports(ProviderCapabilities::PARSE));
    assert!(descriptor.supports(ProviderCapabilities::BRANCH_GRAPH));
    assert!(descriptor.supports(ProviderCapabilities::NATIVE_RESUME));
    assert!(!descriptor.supports(ProviderCapabilities::NATIVE_FORK));
    assert!(!descriptor.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}
