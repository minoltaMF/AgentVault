use std::path::PathBuf;

use provider_workbuddy::{
    NativeOriginProvider, NativeSessionKey, OverlayError, OverlaySource, WorkBuddyActivity,
    WorkBuddyOverlayCatalog, WorkBuddyOverlayObservation,
};

fn native_key(provider: NativeOriginProvider) -> NativeSessionKey {
    NativeSessionKey::new("machine-a", "local", provider, "native-session-1").unwrap()
}

fn activity(id: &str, kind: &str, occurred_at_ms: i64) -> WorkBuddyActivity {
    WorkBuddyActivity::new(id, kind)
        .unwrap()
        .with_status("completed")
        .unwrap()
        .with_summary(format!("{kind} completed"))
        .unwrap()
        .with_occurred_at_ms(occurred_at_ms)
}

#[test]
fn binds_overlay_only_to_the_full_native_identity() {
    let key = native_key(NativeOriginProvider::Claude);
    let observation = WorkBuddyOverlayObservation::new(
        key.clone(),
        OverlaySource::HookManifest,
        1_725_000_000_000,
    )
    .with_harness_id("workbuddy-run-1")
    .unwrap();
    let catalog = WorkBuddyOverlayCatalog::from_observations([observation]).unwrap();

    assert_eq!(catalog.len(), 1);
    assert_eq!(
        catalog
            .overlay_for(&key)
            .unwrap()
            .harness_id
            .as_ref()
            .unwrap()
            .value,
        "workbuddy-run-1"
    );
    assert!(catalog
        .overlay_for(
            &NativeSessionKey::new(
                "machine-b",
                "local",
                NativeOriginProvider::Claude,
                "native-session-1",
            )
            .unwrap()
        )
        .is_none());
    assert!(catalog
        .overlay_for(
            &NativeSessionKey::new(
                "machine-a",
                "remote",
                NativeOriginProvider::Claude,
                "native-session-1",
            )
            .unwrap()
        )
        .is_none());
    assert!(catalog
        .overlay_for(&native_key(NativeOriginProvider::Codex))
        .is_none());
}

#[test]
fn merges_metadata_summary_and_activity_without_becoming_a_transcript() {
    let key = native_key(NativeOriginProvider::Codex);
    let hook = WorkBuddyOverlayObservation::new(
        key.clone(),
        OverlaySource::HookManifest,
        1_725_000_000_000,
    )
    .with_harness_id("workbuddy-run-9")
    .unwrap()
    .with_transcript_path(PathBuf::from("recorded/codex-session.jsonl"))
    .unwrap()
    .with_cwd(PathBuf::from("projects/agentvault"))
    .unwrap()
    .with_model("gpt-test")
    .unwrap()
    .with_activity(activity("activity-1", "tool", 1_724_999_999_900))
    .unwrap();
    let gateway =
        WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 1_725_000_000_500)
            .with_canonical_session_id("canonical-42")
            .unwrap()
            .with_summary("Implemented the overlay boundary.")
            .unwrap()
            .with_activity(activity("activity-2", "message", 1_725_000_000_100))
            .unwrap();

    let forward =
        WorkBuddyOverlayCatalog::from_observations([hook.clone(), gateway.clone()]).unwrap();
    let reversed = WorkBuddyOverlayCatalog::from_observations([gateway, hook]).unwrap();

    assert_eq!(forward, reversed);
    let overlay = forward.overlay_for(&key).unwrap();
    assert_eq!(
        overlay.canonical_session_id.as_ref().unwrap().value,
        "canonical-42"
    );
    assert_eq!(
        overlay.harness_id.as_ref().unwrap().value,
        "workbuddy-run-9"
    );
    assert_eq!(
        overlay.transcript_path.as_ref().unwrap().value,
        PathBuf::from("recorded/codex-session.jsonl")
    );
    assert_eq!(
        overlay.cwd.as_ref().unwrap().value,
        PathBuf::from("projects/agentvault")
    );
    assert_eq!(overlay.model.as_ref().unwrap().value, "gpt-test");
    assert_eq!(
        overlay.summary.as_ref().unwrap().value,
        "Implemented the overlay boundary."
    );
    assert_eq!(
        overlay
            .activities
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>(),
        vec!["activity-1", "activity-2"]
    );
    assert_eq!(overlay.last_observed_at_ms, 1_725_000_000_500);
}

#[test]
fn newer_values_win_and_missing_values_never_erase_evidence() {
    let key = native_key(NativeOriginProvider::Claude);
    let older = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 10)
        .with_summary("older summary")
        .unwrap()
        .with_model("claude-old")
        .unwrap();
    let newer = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 20)
        .with_summary("newer summary")
        .unwrap();

    let catalog = WorkBuddyOverlayCatalog::from_observations([newer, older]).unwrap();
    let overlay = catalog.overlay_for(&key).unwrap();

    assert_eq!(overlay.summary.as_ref().unwrap().value, "newer summary");
    assert_eq!(overlay.model.as_ref().unwrap().value, "claude-old");
}

#[test]
fn gateway_wins_equal_timestamp_ties_independent_of_input_order() {
    let key = native_key(NativeOriginProvider::Codex);
    let manifest = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::HookManifest, 10)
        .with_summary("manifest summary")
        .unwrap();
    let gateway = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 10)
        .with_summary("gateway summary")
        .unwrap();

    let catalog = WorkBuddyOverlayCatalog::from_observations([gateway, manifest]).unwrap();

    let summary = catalog.overlay_for(&key).unwrap().summary.as_ref().unwrap();
    assert_eq!(summary.value, "gateway summary");
    assert_eq!(summary.source, OverlaySource::GatewayApi);
}

#[test]
fn rejects_ambiguous_same_source_observations_and_activity_ids() {
    let key = native_key(NativeOriginProvider::Claude);
    let first = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 10)
        .with_canonical_session_id("canonical-a")
        .unwrap()
        .with_activity(activity("same-activity", "tool", 8))
        .unwrap();
    let second = WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::GatewayApi, 10)
        .with_canonical_session_id("canonical-b")
        .unwrap();
    assert!(matches!(
        WorkBuddyOverlayCatalog::from_observations([first, second]),
        Err(OverlayError::ConflictingObservation {
            field: "canonical_session_id",
            ..
        })
    ));

    let first =
        WorkBuddyOverlayObservation::new(key.clone(), OverlaySource::TranscriptProviderBridge, 11)
            .with_activity(activity("same-activity", "tool", 8))
            .unwrap();
    let second = WorkBuddyOverlayObservation::new(key, OverlaySource::TranscriptProviderBridge, 11)
        .with_activity(activity("same-activity", "message", 8))
        .unwrap();
    assert!(matches!(
        WorkBuddyOverlayCatalog::from_observations([first, second]),
        Err(OverlayError::ConflictingObservation {
            field: "activity",
            ..
        })
    ));
}

#[test]
fn rejects_untrusted_empty_or_control_character_identifiers() {
    assert!(matches!(
        NativeSessionKey::new(
            "",
            "local",
            NativeOriginProvider::Claude,
            "native-session-1"
        ),
        Err(OverlayError::InvalidField {
            field: "machine_id",
            ..
        })
    ));
    assert!(matches!(
        WorkBuddyActivity::new("activity\n2", "tool"),
        Err(OverlayError::InvalidField {
            field: "activity.id",
            ..
        })
    ));

    let too_large = "x".repeat(65_537);
    let result = WorkBuddyOverlayObservation::new(
        native_key(NativeOriginProvider::Codex),
        OverlaySource::GatewayApi,
        10,
    )
    .with_summary(too_large);
    assert!(matches!(
        result,
        Err(OverlayError::InvalidField {
            field: "summary",
            ..
        })
    ));

    let mut activity = WorkBuddyActivity::new("activity-1", "tool").unwrap();
    activity.id = "mutated\nactivity".into();
    let result = WorkBuddyOverlayObservation::new(
        native_key(NativeOriginProvider::Claude),
        OverlaySource::HookManifest,
        10,
    )
    .with_activity(activity);
    assert!(matches!(
        result,
        Err(OverlayError::InvalidField {
            field: "activity.id",
            ..
        })
    ));
}
