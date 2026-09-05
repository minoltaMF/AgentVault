use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use provider_sdk::{
    ConsistencyClass, CwdAvailability, CwdCandidate, CwdCandidateOrigin, NativeSessionKind,
    NativeSessionRef, PreflightCheck, ProviderCapabilities, ProviderDescriptor, ResumeFallback,
    ResumeOptions, ResumePlan, SessionRoot, SessionRootKind, TerminalTarget,
};

#[test]
fn resume_plan_is_a_shell_free_native_command_contract() {
    let descriptor = ProviderDescriptor {
        id: "example".into(),
        display_name: "Example".into(),
        version: "1.0.0".into(),
        native_cli: Some("example-cli".into()),
        capabilities: ProviderCapabilities::NATIVE_RESUME,
        consistency_classes: vec![ConsistencyClass::AppendOnlyJsonl],
        source_kinds: vec![],
        health_probe_timeout_ms: 1_000,
    };
    let source_path = PathBuf::from("/native/session.jsonl");
    let native = NativeSessionRef::new(
        "example",
        Some("session-1".into()),
        &source_path,
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );
    let cwd = PathBuf::from("/work/current");
    let candidate = CwdCandidate {
        project_location_id: Some("location-1".into()),
        path: cwd.clone(),
        origin: CwdCandidateOrigin::ProjectLocation,
        availability: CwdAvailability::Available,
    };
    let options = ResumeOptions {
        cwd_candidates: vec![candidate.clone()],
        selected_cwd: Some(cwd.clone()),
        env_allowlist: BTreeMap::from([("SAFE_FLAG".into(), "1".into())]),
        terminal_target: TerminalTarget::CopyCommand,
    };

    let plan = ResumePlan::native_command(
        &descriptor,
        &native,
        &options,
        vec![OsString::from("--resume"), OsString::from("session-1")],
    )
    .expect("build resume plan");

    assert_eq!(plan.provider_id, "example");
    assert_eq!(plan.native_session_id, "session-1");
    assert_eq!(plan.executable, PathBuf::from("example-cli"));
    assert_eq!(
        plan.args,
        vec![OsString::from("--resume"), OsString::from("session-1")]
    );
    assert_eq!(plan.cwd_candidates, vec![candidate]);
    assert_eq!(plan.selected_cwd, Some(cwd.clone()));
    assert_eq!(plan.env_allowlist.get("SAFE_FLAG"), Some(&"1".into()));
    assert_eq!(plan.terminal_target, TerminalTarget::CopyCommand);
    assert_eq!(
        plan.preflight_checks,
        vec![
            PreflightCheck::ExecutableAvailable(PathBuf::from("example-cli")),
            PreflightCheck::NativeSourceExists(source_path),
            PreflightCheck::SelectedCwdExists(cwd),
        ]
    );
    assert_eq!(plan.fallback, ResumeFallback::ReportUnavailable);
}

#[test]
fn resume_plan_rejects_unregistered_cwd_and_provider_mismatch() {
    let descriptor = ProviderDescriptor {
        id: "example".into(),
        display_name: "Example".into(),
        version: "1.0.0".into(),
        native_cli: Some("example-cli".into()),
        capabilities: ProviderCapabilities::NATIVE_RESUME,
        consistency_classes: vec![],
        source_kinds: vec![],
        health_probe_timeout_ms: 1_000,
    };
    let native = NativeSessionRef::new(
        "other",
        Some("session-1".into()),
        "/native/session.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    assert!(
        ResumePlan::native_command(&descriptor, &native, &ResumeOptions::default(), vec![])
            .is_err()
    );

    let matching = NativeSessionRef {
        provider_id: "example".into(),
        ..native
    };
    let options = ResumeOptions {
        selected_cwd: Some(PathBuf::from("/not/a/candidate")),
        ..ResumeOptions::default()
    };
    assert!(ResumePlan::native_command(&descriptor, &matching, &options, vec![]).is_err());
}
