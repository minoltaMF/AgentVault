use provider_sdk::{
    ConsistencyClass, DetectionContext, DetectionResult, DiscoveryPage, DiscoveryResult,
    ProviderCapabilities, ProviderContext, ProviderDescriptor, ScanCursor, SessionProvider,
    SessionRoot, SourceKind,
};

#[test]
fn capability_bits_are_stable_and_explicit() {
    assert_eq!(ProviderCapabilities::DISCOVER.bits(), 1 << 0);
    assert_eq!(ProviderCapabilities::PARSE.bits(), 1 << 1);
    assert_eq!(ProviderCapabilities::LIVE_HOOK.bits(), 1 << 2);
    assert_eq!(ProviderCapabilities::NATIVE_RESUME.bits(), 1 << 3);
    assert_eq!(ProviderCapabilities::NATIVE_FORK.bits(), 1 << 4);
    assert_eq!(ProviderCapabilities::BACKUP.bits(), 1 << 5);
    assert_eq!(ProviderCapabilities::RESTORE.bits(), 1 << 6);
    assert_eq!(ProviderCapabilities::REPAIR_INDEX.bits(), 1 << 7);
    assert_eq!(ProviderCapabilities::MOVE_CWD.bits(), 1 << 8);
    assert_eq!(ProviderCapabilities::NATIVE_EXPORT.bits(), 1 << 9);
    assert_eq!(ProviderCapabilities::SUBAGENT_LINEAGE.bits(), 1 << 10);
    assert_eq!(ProviderCapabilities::BRANCH_GRAPH.bits(), 1 << 11);
    assert_eq!(ProviderCapabilities::MANAGED_RUNTIME.bits(), 1 << 12);
    assert_eq!(ProviderCapabilities::WRITE_NATIVE_UNSAFE.bits(), 1 << 63);

    let read_capabilities = ProviderCapabilities::DISCOVER | ProviderCapabilities::PARSE;
    assert!(read_capabilities.contains(ProviderCapabilities::DISCOVER));
    assert!(read_capabilities.contains(ProviderCapabilities::PARSE));
    assert!(!read_capabilities.contains(ProviderCapabilities::NATIVE_RESUME));
    assert!(!read_capabilities.contains(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}

#[test]
fn unknown_capability_bits_round_trip_and_remain_detectable() {
    let unknown = 1 << 62;
    let capabilities =
        ProviderCapabilities::from_bits_retain(ProviderCapabilities::DISCOVER.bits() | unknown);

    assert_eq!(
        capabilities.bits(),
        ProviderCapabilities::DISCOVER.bits() | unknown
    );
    assert!(capabilities.contains(ProviderCapabilities::DISCOVER));
    assert!(capabilities.has_unknown_bits());
    assert!(!ProviderCapabilities::DISCOVER.has_unknown_bits());
}

#[test]
fn descriptor_reports_only_declared_capabilities() {
    let descriptor = ProviderDescriptor {
        id: "example".into(),
        display_name: "Example".into(),
        version: "1.0.0".into(),
        native_cli: Some("example-cli".into()),
        capabilities: ProviderCapabilities::DISCOVER | ProviderCapabilities::PARSE,
        consistency_classes: vec![ConsistencyClass::AppendOnlyJsonl],
        source_kinds: vec![SourceKind::Local],
        health_probe_timeout_ms: 1_000,
    };

    assert!(descriptor.supports(ProviderCapabilities::DISCOVER));
    assert!(!descriptor.supports(ProviderCapabilities::BACKUP));
    assert!(!descriptor.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}

struct ExampleProvider;

impl SessionProvider for ExampleProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "example".into(),
            display_name: "Example".into(),
            version: "1.0.0".into(),
            native_cli: None,
            capabilities: ProviderCapabilities::DISCOVER,
            consistency_classes: vec![ConsistencyClass::DirectoryTree],
            source_kinds: vec![SourceKind::ExternalProcess],
            health_probe_timeout_ms: 500,
        }
    }

    fn detect(&self, _context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult> {
        Ok(DetectionResult::from_evidence(Vec::new()))
    }

    fn roots(&self, _context: &ProviderContext<'_>) -> DiscoveryResult<Vec<SessionRoot>> {
        Ok(Vec::new())
    }

    fn discover(
        &self,
        _context: &ProviderContext<'_>,
        _cursor: Option<ScanCursor>,
    ) -> DiscoveryResult<DiscoveryPage> {
        Ok(DiscoveryPage::complete(Vec::new()))
    }
}

#[test]
fn session_provider_is_object_safe_and_uses_its_descriptor() {
    let provider: Box<dyn SessionProvider> = Box::new(ExampleProvider);

    assert_eq!(provider.descriptor().id, "example");
    assert!(provider.supports(ProviderCapabilities::DISCOVER));
    assert!(!provider.supports(ProviderCapabilities::PARSE));
}
