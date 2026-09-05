use registry::{MachineRecord, ProjectLocationRecord, ProjectRecord, Registry, RegistryError};

fn seeded_registry() -> Registry {
    let mut registry = Registry::open_in_memory().expect("open registry");
    registry
        .upsert_machine(&MachineRecord {
            id: "machine-a".into(),
            display_name: "Laptop".into(),
            platform: "macos".into(),
            arch: "aarch64".into(),
            observed_at_ms: 1,
        })
        .expect("upsert machine");
    registry
        .upsert_project(&ProjectRecord {
            id: "project-a".into(),
            display_name: "AgentVault".into(),
            normalized_remotes: vec!["https://example.com/agentvault.git".into()],
            root_commit: Some("abc123".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .expect("upsert project");
    registry
}

fn project_location() -> ProjectLocationRecord {
    ProjectLocationRecord {
        id: "location-a".into(),
        project_id: "project-a".into(),
        machine_id: "machine-a".into(),
        path: "/work/agentvault".into(),
        git_common_dir: Some("/work/agentvault/.git".into()),
        worktree_name: None,
        branch: Some("main".into()),
        first_seen_at_ms: 10,
        last_seen_at_ms: 20,
        status: "available".into(),
    }
}

#[test]
fn project_locations_preserve_path_identity_and_history() {
    let mut registry = seeded_registry();
    let mut location = project_location();
    registry
        .upsert_project_location(&location)
        .expect("insert location");

    location.branch = Some("feature".into());
    location.first_seen_at_ms = 15;
    location.last_seen_at_ms = 30;
    registry
        .upsert_project_location(&location)
        .expect("refresh location");

    let stored = registry
        .project_location("location-a")
        .expect("load location")
        .expect("location exists");
    assert_eq!(stored.first_seen_at_ms, 10);
    assert_eq!(stored.last_seen_at_ms, 30);
    assert_eq!(stored.branch.as_deref(), Some("feature"));
    assert_eq!(
        registry
            .project_locations("project-a")
            .expect("list locations"),
        vec![stored]
    );
}

#[test]
fn project_location_id_cannot_be_rebound_to_another_path() {
    let mut registry = seeded_registry();
    let location = project_location();
    registry
        .upsert_project_location(&location)
        .expect("insert location");
    let rebound = ProjectLocationRecord {
        path: "/different/path".into(),
        ..location
    };

    assert!(matches!(
        registry.upsert_project_location(&rebound),
        Err(RegistryError::IdentityConflict {
            kind: "project location",
            ..
        })
    ));
}

#[test]
fn older_observations_do_not_overwrite_newer_location_state() {
    let mut registry = seeded_registry();
    let mut current = project_location();
    current.last_seen_at_ms = 30;
    current.branch = Some("current".into());
    registry
        .upsert_project_location(&current)
        .expect("insert current location");

    let stale = ProjectLocationRecord {
        last_seen_at_ms: 20,
        branch: Some("stale".into()),
        status: "missing".into(),
        ..current
    };
    registry
        .upsert_project_location(&stale)
        .expect("accept stale observation without regressing state");

    let stored = registry
        .project_location("location-a")
        .expect("load location")
        .expect("location exists");
    assert_eq!(stored.last_seen_at_ms, 30);
    assert_eq!(stored.branch.as_deref(), Some("current"));
    assert_eq!(stored.status, "available");
}
