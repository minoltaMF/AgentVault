use std::str::FromStr;

use chrono::{TimeZone, Utc};
use sha2::{Digest, Sha256};
use vault::{
    ManifestError, ObjectFailureKind, ObjectId, ObjectStore, Sha256Digest, SnapshotConsistency,
    SnapshotManifest, SnapshotManifestDraft, SnapshotManifestStore, SnapshotMember,
    SnapshotStorage, SnapshotVerificationError, SnapshotVerificationIssue, SnapshotVerifier,
};

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::from_str(&value.to_string().repeat(64)).expect("valid digest")
}

fn object(value: char) -> ObjectId {
    ObjectId::sha256(digest(value))
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn manifest(snapshot_id: &str, members: Vec<SnapshotMember>) -> SnapshotManifest {
    SnapshotManifest::try_new(SnapshotManifestDraft {
        snapshot_id: snapshot_id.to_string(),
        machine_id: "machine-local".to_string(),
        created_at: Utc
            .with_ymd_and_hms(2026, 9, 6, 8, 30, 0)
            .single()
            .expect("valid timestamp"),
        reason: "manual".to_string(),
        provider: "claude".to_string(),
        source_instance_id: "claude-home-default".to_string(),
        native_session_id: "native-session".to_string(),
        project_id: None,
        consistency: SnapshotConsistency::Verified,
        parser_version: "claude@1.3.0".to_string(),
        previous_snapshot_id: None,
        resume_plan_at_capture: None,
        git_checkpoint_id: None,
        members,
    })
    .expect("valid manifest")
}

#[test]
fn missing_object_is_reported_without_mutating_the_snapshot() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let missing = object('a');
    let snapshot = manifest(
        "snap_missing",
        vec![SnapshotMember::try_new(
            "settings",
            "state/settings.json",
            12,
            *missing.digest(),
            SnapshotStorage::object(missing),
        )
        .expect("member")],
    );
    let manifest_path = manifests.write_new(&snapshot).expect("write manifest");
    let manifest_bytes = std::fs::read(&manifest_path).expect("manifest bytes");

    let report = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_missing")
        .expect("verification report");

    assert!(!report.is_verified());
    assert_eq!(report.snapshot_id(), "snap_missing");
    assert_eq!(report.member_count(), 1);
    assert_eq!(report.object_reference_count(), 1);
    assert_eq!(
        report.issues(),
        &[SnapshotVerificationIssue::ObjectUnavailable {
            member_path: "state/settings.json".to_string(),
            object_id: missing,
            failure: ObjectFailureKind::Missing,
        }]
    );
    assert_eq!(
        std::fs::read(manifest_path).expect("manifest remains readable"),
        manifest_bytes
    );
}

#[test]
fn reconstructed_member_size_mismatch_is_reported() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let first = objects.put_bytes(b"hello ").expect("first chunk");
    let second = objects.put_bytes(b"world").expect("second chunk");
    let snapshot = manifest(
        "snap_size_mismatch",
        vec![SnapshotMember::try_new(
            "transcript",
            "sessions/native-session.jsonl",
            10,
            sha256(b"hello world"),
            SnapshotStorage::chunked(vec![*first.id(), *second.id()]).expect("chunks"),
        )
        .expect("member")],
    );
    manifests.write_new(&snapshot).expect("write manifest");

    let report = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_size_mismatch")
        .expect("verification report");

    assert!(!report.is_verified());
    assert_eq!(
        report.issues(),
        &[SnapshotVerificationIssue::MemberSizeMismatch {
            member_path: "sessions/native-session.jsonl".to_string(),
            expected: 10,
            actual: 11,
        }]
    );
}

#[test]
fn reconstructed_member_digest_mismatch_is_reported() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let first = objects.put_bytes(b"hello ").expect("first chunk");
    let second = objects.put_bytes(b"world").expect("second chunk");
    let expected = digest('f');
    let actual = sha256(b"hello world");
    let snapshot = manifest(
        "snap_digest_mismatch",
        vec![SnapshotMember::try_new(
            "transcript",
            "sessions/native-session.jsonl",
            11,
            expected,
            SnapshotStorage::chunked(vec![*first.id(), *second.id()]).expect("chunks"),
        )
        .expect("member")],
    );
    manifests.write_new(&snapshot).expect("write manifest");

    let report = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_digest_mismatch")
        .expect("verification report");

    assert!(!report.is_verified());
    assert_eq!(
        report.issues(),
        &[SnapshotVerificationIssue::MemberDigestMismatch {
            member_path: "sessions/native-session.jsonl".to_string(),
            expected,
            actual,
        }]
    );
}

#[test]
fn corrupt_object_is_reported_and_left_untouched() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let stored = objects.put_bytes(b"protected").expect("store object");
    let object_path = objects.object_path(stored.id());
    let corrupt = b"corrupt envelope";
    std::fs::write(&object_path, corrupt).expect("corrupt test object");
    let snapshot = manifest(
        "snap_corrupt",
        vec![SnapshotMember::try_new(
            "transcript",
            "sessions/native-session.jsonl",
            9,
            *stored.id().digest(),
            SnapshotStorage::object(*stored.id()),
        )
        .expect("member")],
    );
    manifests.write_new(&snapshot).expect("write manifest");

    let report = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_corrupt")
        .expect("verification report");

    assert!(!report.is_verified());
    assert_eq!(
        report.issues(),
        &[SnapshotVerificationIssue::ObjectUnavailable {
            member_path: "sessions/native-session.jsonl".to_string(),
            object_id: *stored.id(),
            failure: ObjectFailureKind::Corrupt,
        }]
    );
    assert_eq!(
        std::fs::read(object_path).expect("corrupt object remains readable"),
        corrupt
    );
}

#[test]
fn damaged_compressed_payload_is_classified_as_corrupt() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let stored = objects
        .put_bytes(b"compressed payload integrity")
        .expect("store object");
    let object_path = objects.object_path(stored.id());
    let mut corrupt = std::fs::read(&object_path).expect("encoded object");
    *corrupt.last_mut().expect("zlib checksum") ^= 0xff;
    std::fs::write(&object_path, &corrupt).expect("damage compressed payload");
    let snapshot = manifest(
        "snap_bad_payload",
        vec![SnapshotMember::try_new(
            "transcript",
            "sessions/native-session.jsonl",
            28,
            *stored.id().digest(),
            SnapshotStorage::object(*stored.id()),
        )
        .expect("member")],
    );
    manifests.write_new(&snapshot).expect("write manifest");

    let report = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_bad_payload")
        .expect("corruption report");

    assert_eq!(
        report.issues(),
        &[SnapshotVerificationIssue::ObjectUnavailable {
            member_path: "sessions/native-session.jsonl".to_string(),
            object_id: *stored.id(),
            failure: ObjectFailureKind::Corrupt,
        }]
    );
    assert_eq!(
        std::fs::read(object_path).expect("damaged object remains readable"),
        corrupt
    );
}

#[test]
fn whole_and_chunked_members_verify_without_mutating_vault_files() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let settings = objects.put_bytes(b"settings").expect("settings object");
    let first = objects.put_bytes(b"hello ").expect("first chunk");
    let second = objects.put_bytes(b"world").expect("second chunk");
    let snapshot = manifest(
        "snap_verified",
        vec![
            SnapshotMember::try_new(
                "settings",
                "state/settings.json",
                8,
                sha256(b"settings"),
                SnapshotStorage::object(*settings.id()),
            )
            .expect("settings member"),
            SnapshotMember::try_new(
                "transcript",
                "sessions/native-session.jsonl",
                11,
                sha256(b"hello world"),
                SnapshotStorage::chunked(vec![*first.id(), *second.id()]).expect("chunks"),
            )
            .expect("transcript member"),
        ],
    );
    let manifest_path = manifests.write_new(&snapshot).expect("write manifest");
    let files = [
        manifest_path,
        objects.object_path(settings.id()),
        objects.object_path(first.id()),
        objects.object_path(second.id()),
    ];
    let before = files
        .iter()
        .map(|path| std::fs::read(path).expect("vault bytes"))
        .collect::<Vec<_>>();

    let verifier = SnapshotVerifier::new(&manifests, &objects);
    let report = verifier.verify("snap_verified").expect("verified snapshot");
    let repeated = verifier
        .verify("snap_verified")
        .expect("repeat verification");

    assert!(report.is_verified());
    assert_eq!(report.snapshot_id(), "snap_verified");
    assert_eq!(report.member_count(), 2);
    assert_eq!(report.object_reference_count(), 3);
    assert!(report.issues().is_empty());
    assert_eq!(repeated, report);
    assert_eq!(
        files
            .iter()
            .map(|path| std::fs::read(path).expect("unchanged vault bytes"))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn malformed_manifest_is_rejected_instead_of_reported_as_verified() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    let objects_root = temp.path().join("objects");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    std::fs::create_dir(&objects_root).expect("objects root");
    let manifests = SnapshotManifestStore::open(&snapshots_root).expect("manifest store");
    let objects = ObjectStore::open(&objects_root).expect("object store");
    let stored = objects.put_bytes(b"settings").expect("settings object");
    let snapshot = manifest(
        "snap_bad_manifest",
        vec![SnapshotMember::try_new(
            "settings",
            "state/settings.json",
            8,
            *stored.id().digest(),
            SnapshotStorage::object(*stored.id()),
        )
        .expect("member")],
    );
    let manifest_path = manifests.write_new(&snapshot).expect("write manifest");
    let corrupt = br#"{"schema_version":1,"snapshot_id":"snap_bad_manifest""#;
    std::fs::write(&manifest_path, corrupt).expect("corrupt test manifest");

    let error = SnapshotVerifier::new(&manifests, &objects)
        .verify("snap_bad_manifest")
        .expect_err("malformed manifest must fail verification");

    assert!(matches!(
        error,
        SnapshotVerificationError::Manifest(ManifestError::Json(_))
    ));
    assert_eq!(
        std::fs::read(manifest_path).expect("corrupt manifest remains readable"),
        corrupt
    );
}
