use std::path::{Path, PathBuf};

use provider_claude::ClaudeProvider;
use provider_sdk::{
    ConsistencyClass, CwdAvailability, CwdCandidateOrigin, NativeSessionKind, NativeSessionRef,
    SessionRoot, SessionRootKind, TerminalTarget,
};
use registry::ProjectLocationRecord;
use resume::{map_cwd, plan_native_resume, CwdMappingRequest, CwdSelection};

fn location(
    id: &str,
    project_id: &str,
    machine_id: &str,
    path: &str,
    status: &str,
    last_seen_at_ms: i64,
) -> ProjectLocationRecord {
    ProjectLocationRecord {
        id: id.into(),
        project_id: project_id.into(),
        machine_id: machine_id.into(),
        path: path.into(),
        git_common_dir: None,
        worktree_name: None,
        branch: None,
        first_seen_at_ms: 1,
        last_seen_at_ms,
        status: status.into(),
    }
}

#[test]
fn cwd_mapping_uses_only_the_same_project_and_machine() {
    let locations = vec![
        location(
            "current",
            "project-a",
            "machine-a",
            "/new/repo",
            "available",
            30,
        ),
        location(
            "missing",
            "project-a",
            "machine-a",
            "/older/repo",
            "missing",
            20,
        ),
        location(
            "other-machine",
            "project-a",
            "machine-b",
            "/remote/repo",
            "available",
            40,
        ),
        location(
            "other-project",
            "project-b",
            "machine-a",
            "/wrong/repo",
            "available",
            50,
        ),
    ];

    let mapping = map_cwd(CwdMappingRequest {
        current_machine_id: "machine-a",
        project_id: Some("project-a"),
        cwd_at_start: Some(Path::new("/old/repo")),
        project_locations: &locations,
        selection: CwdSelection::Automatic,
    })
    .expect("map cwd");

    assert_eq!(mapping.candidates.len(), 3);
    assert_eq!(mapping.candidates[0].path, PathBuf::from("/old/repo"));
    assert_eq!(
        mapping.candidates[0].origin,
        CwdCandidateOrigin::SessionStart
    );
    assert_eq!(mapping.candidates[0].availability, CwdAvailability::Unknown);
    assert_eq!(mapping.candidates[1].path, PathBuf::from("/new/repo"));
    assert_eq!(
        mapping.candidates[1].availability,
        CwdAvailability::Available
    );
    assert_eq!(mapping.candidates[2].path, PathBuf::from("/older/repo"));
    assert_eq!(mapping.candidates[2].availability, CwdAvailability::Missing);
    assert_eq!(mapping.selected_cwd, Some(PathBuf::from("/new/repo")));
}

#[test]
fn automatic_mapping_requires_one_unambiguous_available_location() {
    let locations = vec![
        location(
            "one",
            "project-a",
            "machine-a",
            "/repo/one",
            "available",
            30,
        ),
        location(
            "two",
            "project-a",
            "machine-a",
            "/repo/two",
            "available",
            20,
        ),
    ];

    let ambiguous = map_cwd(CwdMappingRequest {
        current_machine_id: "machine-a",
        project_id: Some("project-a"),
        cwd_at_start: Some(Path::new("/old/repo")),
        project_locations: &locations,
        selection: CwdSelection::Automatic,
    })
    .expect("map ambiguous cwd");
    assert_eq!(ambiguous.selected_cwd, None);

    let exact = map_cwd(CwdMappingRequest {
        cwd_at_start: Some(Path::new("/repo/two")),
        ..CwdMappingRequest {
            current_machine_id: "machine-a",
            project_id: Some("project-a"),
            cwd_at_start: None,
            project_locations: &locations,
            selection: CwdSelection::Automatic,
        }
    })
    .expect("map exact cwd");
    assert_eq!(exact.selected_cwd, Some(PathBuf::from("/repo/two")));
}

#[test]
fn explicit_mapping_rejects_missing_or_foreign_locations() {
    let locations = vec![
        location(
            "current",
            "project-a",
            "machine-a",
            "/new/repo",
            "available",
            30,
        ),
        location(
            "missing",
            "project-a",
            "machine-a",
            "/old/repo",
            "missing",
            20,
        ),
        location(
            "foreign",
            "project-a",
            "machine-b",
            "/remote/repo",
            "available",
            40,
        ),
    ];

    for selected in ["missing", "foreign", "unknown"] {
        assert!(map_cwd(CwdMappingRequest {
            current_machine_id: "machine-a",
            project_id: Some("project-a"),
            cwd_at_start: None,
            project_locations: &locations,
            selection: CwdSelection::ProjectLocation(selected),
        })
        .is_err());
    }

    let explicit = map_cwd(CwdMappingRequest {
        current_machine_id: "machine-a",
        project_id: Some("project-a"),
        cwd_at_start: None,
        project_locations: &locations,
        selection: CwdSelection::ProjectLocation("current"),
    })
    .expect("select current location");
    assert_eq!(explicit.selected_cwd, Some(PathBuf::from("/new/repo")));
}

#[test]
fn historical_session_cwd_requires_explicit_selection_when_unverified() {
    let mapping = map_cwd(CwdMappingRequest {
        current_machine_id: "machine-a",
        project_id: None,
        cwd_at_start: Some(Path::new("/old/repo")),
        project_locations: &[],
        selection: CwdSelection::Automatic,
    })
    .expect("map unverified historical cwd");
    assert_eq!(mapping.selected_cwd, None);

    let selected = map_cwd(CwdMappingRequest {
        current_machine_id: "machine-a",
        project_id: None,
        cwd_at_start: Some(Path::new("/old/repo")),
        project_locations: &[],
        selection: CwdSelection::SessionStart,
    })
    .expect("select historical cwd explicitly");
    assert_eq!(selected.selected_cwd, Some(PathBuf::from("/old/repo")));
}

#[test]
fn native_resume_engine_composes_mapping_with_provider_plan() {
    let locations = vec![location(
        "current",
        "project-a",
        "machine-a",
        "/new/repo",
        "available",
        30,
    )];
    let native = NativeSessionRef::new(
        "claude",
        Some("session-1".into()),
        "/native/session.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    let plan = plan_native_resume(
        &ClaudeProvider,
        &native,
        CwdMappingRequest {
            current_machine_id: "machine-a",
            project_id: Some("project-a"),
            cwd_at_start: Some(Path::new("/old/repo")),
            project_locations: &locations,
            selection: CwdSelection::Automatic,
        },
        TerminalTarget::CopyCommand,
    )
    .expect("compose native resume plan");

    assert_eq!(plan.selected_cwd, Some(PathBuf::from("/new/repo")));
    assert_eq!(plan.cwd_candidates.len(), 2);
    assert_eq!(plan.terminal_target, TerminalTarget::CopyCommand);
    assert!(plan.env_allowlist.is_empty());
}
