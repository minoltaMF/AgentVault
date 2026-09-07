use std::sync::{Arc, Barrier};

use chrono::{TimeZone, Utc};
use vault::{
    ObjectStore, SnapshotConsistency, SnapshotManifest, SnapshotManifestDraft,
    SnapshotManifestStore, SnapshotMember, SnapshotStorage, SnapshotVerifier,
};

fn manifest(
    snapshot_id: &str,
    reason: &str,
    object: vault::ObjectId,
    size: u64,
) -> SnapshotManifest {
    SnapshotManifest::try_new(SnapshotManifestDraft {
        snapshot_id: snapshot_id.into(),
        machine_id: "test-machine".into(),
        created_at: Utc
            .with_ymd_and_hms(2026, 9, 7, 12, 0, 0)
            .single()
            .expect("valid timestamp"),
        reason: reason.into(),
        provider: "claude".into(),
        source_instance_id: "test-source".into(),
        native_session_id: "test-native-session".into(),
        project_id: None,
        consistency: SnapshotConsistency::Verified,
        parser_version: "test-parser-v1".into(),
        previous_snapshot_id: None,
        resume_plan_at_capture: None,
        git_checkpoint_id: None,
        members: vec![SnapshotMember::try_new(
            "transcript",
            "sessions/test-native-session.jsonl",
            size,
            *object.digest(),
            SnapshotStorage::object(object),
        )
        .expect("valid member")],
    })
    .expect("valid manifest")
}

#[test]
fn object_survives_interruption_before_manifest_publish() {
    let temp = tempfile::tempdir().expect("temporary safety matrix");
    let source = temp.path().join("source-session.jsonl");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let source_bytes = b"{\"type\":\"user\",\"text\":\"sanitized fixture\"}\n";
    std::fs::write(&source, source_bytes).expect("test source");

    let orphan_id = {
        let objects = ObjectStore::open(&objects_root).expect("object store");
        *objects
            .put_bytes(source_bytes)
            .expect("durable object")
            .id()
    };

    // Simulate process loss after the object commit but before manifest publication.
    let objects = ObjectStore::open(&objects_root).expect("reopen object store");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    assert!(manifests.read("snap_interrupted").is_err());
    assert!(std::fs::read_dir(&snapshots_root)
        .expect("snapshots remain inspectable")
        .next()
        .is_none());
    assert_eq!(
        std::fs::read(&source).expect("source remains"),
        source_bytes
    );
    assert_eq!(
        objects.read(&orphan_id).expect("recover object"),
        source_bytes
    );

    let stored = objects
        .put_bytes(source_bytes)
        .expect("reuse orphan object");
    assert_eq!(*stored.id(), orphan_id);
    let snapshot = manifest(
        "snap_interrupted",
        "startup-reconcile-fixture",
        *stored.id(),
        source_bytes.len() as u64,
    );
    manifests.write_new(&snapshot).expect("publish manifest");
    assert!(SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_interrupted")
        .expect("verification report")
        .is_verified());
}

#[test]
fn concurrent_conflicting_manifest_publish_keeps_one_verified_winner() {
    let temp = tempfile::tempdir().expect("temporary safety matrix");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let first_bytes = b"first sanitized snapshot\n";
    let second_bytes = b"second sanitized snapshot\n";
    let first_object = objects.put_bytes(first_bytes).expect("first object");
    let second_object = objects.put_bytes(second_bytes).expect("second object");
    let first = manifest(
        "snap_race",
        "first-writer",
        *first_object.id(),
        first_bytes.len() as u64,
    );
    let second = manifest(
        "snap_race",
        "second-writer",
        *second_object.id(),
        second_bytes.len() as u64,
    );
    let expected = [first.clone(), second.clone()];
    let manifests = Arc::new(SnapshotManifestStore::open(&snapshots_root).expect("manifest store"));
    let barrier = Arc::new(Barrier::new(2));

    let writers = [first, second].map(|candidate| {
        let manifests = Arc::clone(&manifests);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            manifests.write_new(&candidate)
        })
    });
    let results = writers.map(|writer| writer.join().expect("manifest writer"));

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let persisted = manifests.read("snap_race").expect("winning manifest");
    assert!(expected.contains(&persisted));
    assert!(SnapshotVerifier::new(&manifests, &objects)
        .verify_manifest(&persisted)
        .expect("verification report")
        .is_verified());
    assert_eq!(
        objects.read(first_object.id()).expect("first remains"),
        first_bytes
    );
    assert_eq!(
        objects.read(second_object.id()).expect("second remains"),
        second_bytes
    );
    assert_eq!(
        std::fs::read_dir(snapshots_root.join("snap_race"))
            .expect("snapshot directory")
            .count(),
        1,
        "failed publisher must not leave a temporary manifest"
    );
}
