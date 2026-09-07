use std::path::{Path, PathBuf};
use std::time::Duration;

use health::{
    inspect_provider, CliObservation, CliProbe, CliProbeStatus, DiagnosticSeverity, DoctorState,
    ProviderDiagnosticCode, RuntimeStatus,
};
use provider_sdk::{
    ConsistencyClass, DetectionContext, DetectionResult, DiscoveryError, DiscoveryPage,
    DiscoveryResult, ProviderCapabilities, ProviderContext, ProviderDescriptor, ScanCursor,
    SessionProvider, SessionRoot, SessionRootKind, SourceKind,
};

struct FakeProvider {
    fail_detection: bool,
    fail_roots: bool,
}

impl SessionProvider for FakeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "example".into(),
            display_name: "Example Agent".into(),
            version: "2.3.4".into(),
            native_cli: Some("example-agent".into()),
            capabilities: ProviderCapabilities::DISCOVER
                | ProviderCapabilities::LIVE_HOOK
                | ProviderCapabilities::NATIVE_RESUME
                | ProviderCapabilities::REPAIR_INDEX,
            consistency_classes: vec![ConsistencyClass::AppendOnlyJsonl],
            source_kinds: vec![SourceKind::Local],
            health_probe_timeout_ms: 250,
        }
    }

    fn detect(&self, context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult> {
        if self.fail_detection {
            return Err(DiscoveryError::unsafe_path(
                context.root(),
                "test detection failure",
            ));
        }
        Ok(DetectionResult::from_evidence(vec![context
            .root()
            .join("sessions")]))
    }

    fn roots(&self, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<SessionRoot>> {
        if self.fail_roots {
            return Err(DiscoveryError::traversal(
                context.root(),
                "test root failure",
            ));
        }
        Ok(vec![SessionRoot::new(
            context.root().join("sessions"),
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        )])
    }

    fn discover(
        &self,
        _context: &ProviderContext<'_>,
        _cursor: Option<ScanCursor>,
    ) -> DiscoveryResult<DiscoveryPage> {
        panic!("provider doctor must not scan native session contents")
    }
}

struct FakeCliProbe {
    observation: CliObservation,
}

impl CliProbe for FakeCliProbe {
    fn probe(&self, executable: &str, timeout: Duration) -> CliObservation {
        assert_eq!(executable, "example-agent");
        assert_eq!(timeout, Duration::from_millis(250));
        self.observation.clone()
    }
}

fn doctor_state(root: &Path) -> DoctorState<'_> {
    DoctorState {
        config_root: root,
        observed_at_ms: 1_725_000_000_100,
        hook_status: RuntimeStatus::Active,
        watcher_status: RuntimeStatus::Inactive,
        last_reconciliation_ms: Some(1_725_000_000_000),
        parser_version: Some("agentvault-parser/7"),
        unknown_event_count: 3,
    }
}

#[test]
fn doctor_reports_required_fields_without_discovering_or_mutating_sessions() {
    let provider = FakeProvider {
        fail_detection: false,
        fail_roots: false,
    };
    let probe = FakeCliProbe {
        observation: CliObservation::available("example-agent 9.1.0"),
    };
    let config_root = PathBuf::from("test-fixtures/provider-home");

    let report = inspect_provider(&provider, doctor_state(&config_root), &probe);

    assert_eq!(report.provider_id, "example");
    assert_eq!(report.provider_display_name, "Example Agent");
    assert_eq!(report.provider_version, "2.3.4");
    assert_eq!(report.config_root, config_root);
    assert_eq!(report.observed_at_ms, 1_725_000_000_100);
    assert_eq!(report.cli.status, CliProbeStatus::Available);
    assert_eq!(report.cli.version.as_deref(), Some("example-agent 9.1.0"));
    assert_eq!(report.detected, Some(true));
    assert_eq!(report.detection_evidence.len(), 1);
    assert_eq!(report.session_roots.len(), 1);
    assert_eq!(report.hook_status, RuntimeStatus::Active);
    assert_eq!(report.watcher_status, RuntimeStatus::Inactive);
    assert_eq!(report.last_reconciliation_ms, Some(1_725_000_000_000));
    assert_eq!(
        report.parser_version.as_deref(),
        Some("agentvault-parser/7")
    );
    assert_eq!(report.unknown_event_count, 3);
    assert!(report.resume_capability);
    assert!(report.repair_capability);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ProviderDiagnosticCode::UnknownEvents
            && diagnostic.severity == DiagnosticSeverity::Warning
    }));
}

#[test]
fn doctor_contains_probe_and_provider_failures_as_structured_diagnostics() {
    let provider = FakeProvider {
        fail_detection: true,
        fail_roots: true,
    };
    let probe = FakeCliProbe {
        observation: CliObservation::timed_out(),
    };
    let root = PathBuf::from("test-fixtures/unreadable-provider-home");

    let report = inspect_provider(&provider, doctor_state(&root), &probe);

    assert_eq!(report.detected, None);
    assert!(report.session_roots.is_empty());
    let codes = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&ProviderDiagnosticCode::CliProbeTimedOut));
    assert!(codes.contains(&ProviderDiagnosticCode::DetectionFailed));
    assert!(codes.contains(&ProviderDiagnosticCode::SessionRootsFailed));
}
