use std::path::PathBuf;

use app_service::{build_recovery_inbox, RecoveryInboxKind, RecoverySignal, RecoveryUrgency};
use health::{
    CliObservation, DiagnosticSeverity, ProviderDiagnostic, ProviderDiagnosticCode,
    ProviderDoctorReport, RuntimeStatus,
};

fn signal(
    key: &str,
    kind: RecoveryInboxKind,
    urgency: RecoveryUrgency,
    observed_at_ms: i64,
) -> RecoverySignal {
    RecoverySignal::new(
        key,
        kind,
        urgency,
        observed_at_ms,
        "Needs attention",
        "Inspect the evidence before taking action.",
    )
    .expect("valid recovery signal")
}

fn doctor_report() -> ProviderDoctorReport {
    ProviderDoctorReport {
        provider_id: "codex".into(),
        provider_display_name: "Codex".into(),
        provider_version: "0.1.0".into(),
        config_root: PathBuf::from(".codex"),
        observed_at_ms: 100,
        cli: CliObservation::available("codex-cli 1.2.3"),
        detected: Some(true),
        detection_evidence: Vec::new(),
        session_roots: Vec::new(),
        hook_status: RuntimeStatus::NotApplicable,
        watcher_status: RuntimeStatus::Active,
        last_reconciliation_ms: Some(42),
        parser_version: Some("codex-parser/1".into()),
        unknown_event_count: 0,
        resume_capability: true,
        repair_capability: false,
        diagnostics: vec![
            ProviderDiagnostic::new(
                ProviderDiagnosticCode::NativeIndexInvisible,
                DiagnosticSeverity::Warning,
                "A native session is missing from the provider index.",
                "Review a repair preview before changing the native index.",
            ),
            ProviderDiagnostic::new(
                ProviderDiagnosticCode::ReconciliationNotRun,
                DiagnosticSeverity::Info,
                "Reconciliation has not run yet.",
                "Run a read-only reconciliation.",
            ),
        ],
    }
}

#[test]
fn inbox_prioritizes_actionable_items_deduplicates_and_is_deterministic() {
    let items = vec![
        signal(
            "session:a",
            RecoveryInboxKind::ActiveSessionWithoutCheckpoint,
            RecoveryUrgency::Warning,
            10,
        ),
        signal(
            "restore:b",
            RecoveryInboxKind::RestoreConflict,
            RecoveryUrgency::Error,
            5,
        ),
        signal(
            "session:a",
            RecoveryInboxKind::ActiveSessionWithoutCheckpoint,
            RecoveryUrgency::Warning,
            20,
        ),
        signal(
            "project:c",
            RecoveryInboxKind::ProjectLocationMoved,
            RecoveryUrgency::Info,
            30,
        ),
    ];

    let first = build_recovery_inbox(items.clone(), [&doctor_report()]);
    let mut reversed = items;
    reversed.reverse();
    let second = build_recovery_inbox(reversed, [&doctor_report()]);

    assert_eq!(first, second);
    assert_eq!(first.items.len(), 3);
    assert_eq!(first.items[0].kind, RecoveryInboxKind::RestoreConflict);
    assert_eq!(first.items[1].observed_at_ms, 20);
    assert_eq!(first.items[2].kind, RecoveryInboxKind::NativeIndexInvisible);
    assert_eq!(first.error_count, 1);
    assert_eq!(first.warning_count, 2);
    assert!(first.items.iter().all(|item| item.urgency.is_actionable()));
}

#[test]
fn recovery_signal_rejects_ambiguous_identity_or_empty_guidance() {
    assert!(RecoverySignal::new(
        " ",
        RecoveryInboxKind::CleanupDeadlineNear,
        RecoveryUrgency::Warning,
        1,
        "Title",
        "Action",
    )
    .is_err());
    assert!(RecoverySignal::new(
        "session:one",
        RecoveryInboxKind::CleanupDeadlineNear,
        RecoveryUrgency::Warning,
        1,
        "Title",
        " ",
    )
    .is_err());
}

#[test]
fn provider_health_is_not_silently_lost_when_external_metadata_is_malformed() {
    let mut report = doctor_report();
    report.provider_id = "bad\nprovider".into();
    report.diagnostics = vec![ProviderDiagnostic::new(
        ProviderDiagnosticCode::CliMissing,
        DiagnosticSeverity::Warning,
        " ",
        " ",
    )];

    let inbox = build_recovery_inbox(Vec::new(), [&report]);

    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].kind, RecoveryInboxKind::ProviderHealth);
    assert!(!inbox.items[0].key.chars().any(char::is_control));
    assert!(!inbox.items[0].title.is_empty());
    assert!(!inbox.items[0].recommended_action.is_empty());
}
