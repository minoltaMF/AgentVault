use std::path::Path;
use std::sync::atomic::AtomicBool;

use provider_sdk::{
    ConsistencyClass, DetectionContext, DiscoveryPage, NativeSessionKind, NativeSessionRef,
    ProviderContext, ScanCursor, SessionRoot, SessionRootKind,
};

#[test]
fn discovery_context_carries_only_root_and_optional_cancellation() {
    let root = Path::new("provider-home");
    let cancel = AtomicBool::new(false);

    let detection = DetectionContext::new(root);
    let context = ProviderContext::new(root).with_cancellation(&cancel);

    assert_eq!(detection.root(), root);
    assert_eq!(context.root(), root);
    assert!(!context.is_cancelled());
}

#[test]
fn complete_page_preserves_native_identity_and_provenance() {
    let root = SessionRoot::new(
        Path::new("provider-home/sessions"),
        SessionRootKind::Active,
        ConsistencyClass::AppendOnlyJsonl,
    );
    let native = NativeSessionRef::new(
        "example",
        Some("native-1".into()),
        Path::new("provider-home/sessions/native-1.jsonl"),
        root.clone(),
        NativeSessionKind::Primary,
    )
    .with_parent_native_session_id("parent-1");

    let page = DiscoveryPage::complete(vec![native.clone()]);

    assert_eq!(page.sessions, vec![native]);
    assert_eq!(page.sessions[0].session_root, root);
    assert_eq!(
        page.sessions[0].parent_native_session_id.as_deref(),
        Some("parent-1")
    );
    assert!(page.next_cursor.is_none());
    assert_eq!(ScanCursor::new("next").as_str(), "next");
}
