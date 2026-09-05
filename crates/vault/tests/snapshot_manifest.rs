use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use provider_sdk::{
    CwdAvailability, CwdCandidate, CwdCandidateOrigin, PreflightCheck, ResumeFallback, ResumePlan,
    TerminalTarget,
};
use serde_json::{json, Value};
use vault::{
    CapturedResumePlan, ManifestError, ObjectId, Sha256Digest, SnapshotConsistency,
    SnapshotManifest, SnapshotManifestDraft, SnapshotManifestStore, SnapshotMember,
    SnapshotStorage, SNAPSHOT_MANIFEST_SCHEMA_VERSION,
};

fn digest(fill: char) -> Sha256Digest {
    Sha256Digest::from_str(&fill.to_string().repeat(64)).expect("valid test digest")
}

fn object(fill: char) -> ObjectId {
    ObjectId::sha256(digest(fill))
}

fn captured_resume_plan() -> CapturedResumePlan {
    let plan = ResumePlan {
        provider_id: "claude".to_string(),
        native_session_id: "native-session".to_string(),
        executable: PathBuf::from("claude"),
        args: vec![OsString::from("--resume"), OsString::from("native-session")],
        cwd_candidates: vec![CwdCandidate {
            project_location_id: Some("location-primary".to_string()),
            path: PathBuf::from("/workspace/AgentVault"),
            origin: CwdCandidateOrigin::ProjectLocation,
            availability: CwdAvailability::Available,
        }],
        selected_cwd: Some(PathBuf::from("/workspace/AgentVault")),
        env_allowlist: BTreeMap::from([("SAFE_FLAG".to_string(), "1".to_string())]),
        terminal_target: TerminalTarget::CopyCommand,
        preflight_checks: vec![PreflightCheck::ExecutableAvailable(PathBuf::from("claude"))],
        fallback: ResumeFallback::ReportUnavailable,
    };

    CapturedResumePlan::try_from(&plan).expect("UTF-8 resume plan")
}

fn manifest() -> SnapshotManifest {
    let transcript_hash = digest('a');
    let settings_hash = digest('d');
    SnapshotManifest::try_new(SnapshotManifestDraft {
        snapshot_id: "snap_01JTEST".to_string(),
        machine_id: "machine-local".to_string(),
        created_at: DateTime::parse_from_rfc3339("2026-09-04T00:15:00Z")
            .expect("valid time")
            .with_timezone(&Utc),
        reason: "periodic_active_checkpoint".to_string(),
        provider: "claude".to_string(),
        source_instance_id: "claude-home-default".to_string(),
        native_session_id: "native-session".to_string(),
        project_id: Some("project-agentvault".to_string()),
        consistency: SnapshotConsistency::VerifiedPrefix,
        parser_version: "claude@1.3.0".to_string(),
        previous_snapshot_id: Some("snap_01JPREVIOUS".to_string()),
        resume_plan_at_capture: Some(captured_resume_plan()),
        git_checkpoint_id: Some("git_01JTEST".to_string()),
        members: vec![
            SnapshotMember::try_new(
                "settings",
                r"state\settings.json",
                42,
                settings_hash,
                SnapshotStorage::object(ObjectId::sha256(settings_hash)),
            )
            .expect("valid object member"),
            SnapshotMember::try_new(
                "transcript",
                "projects/example/native-session.jsonl",
                8_388_608,
                transcript_hash,
                SnapshotStorage::chunked(vec![object('b'), object('c')]).expect("non-empty chunks"),
            )
            .expect("valid chunked member"),
        ],
    })
    .expect("valid manifest")
}

#[test]
fn manifest_v1_serializes_the_approved_schema_in_stable_member_order() {
    let manifest = manifest();
    assert_eq!(manifest.schema_version(), SNAPSHOT_MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.members()[0].role(), "transcript");
    assert_eq!(manifest.members()[1].relative_path(), "state/settings.json");

    let value: Value = serde_json::from_slice(&manifest.to_json_bytes().expect("serialize"))
        .expect("manifest JSON");
    assert_eq!(
        value,
        json!({
            "schema_version": 1,
            "snapshot_id": "snap_01JTEST",
            "machine_id": "machine-local",
            "created_at": "2026-09-04T00:15:00Z",
            "reason": "periodic_active_checkpoint",
            "provider": "claude",
            "source_instance_id": "claude-home-default",
            "native_session_id": "native-session",
            "project_id": "project-agentvault",
            "consistency": "verified_prefix",
            "parser_version": "claude@1.3.0",
            "previous_snapshot_id": "snap_01JPREVIOUS",
            "resume_plan_at_capture": {
                "executable": "claude",
                "args": ["--resume", "native-session"],
                "cwd": "/workspace/AgentVault"
            },
            "git_checkpoint_id": "git_01JTEST",
            "members": [
                {
                    "role": "transcript",
                    "relative_path": "projects/example/native-session.jsonl",
                    "size": 8388608,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "storage": {
                        "kind": "chunked",
                        "chunks": [
                            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        ]
                    }
                },
                {
                    "role": "settings",
                    "relative_path": "state/settings.json",
                    "size": 42,
                    "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "storage": {
                        "kind": "object",
                        "object": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    }
                }
            ]
        })
    );
    assert!(value["resume_plan_at_capture"]
        .get("env_allowlist")
        .is_none());
}

#[test]
fn manifest_json_round_trip_is_deterministic_and_revalidates_input() {
    let original = manifest();
    let bytes = original.to_json_bytes().expect("serialize");
    let decoded = SnapshotManifest::from_json_slice(&bytes).expect("parse");

    assert_eq!(decoded, original);
    assert_eq!(decoded.to_json_bytes().expect("reserialize"), bytes);

    let mut unsupported: Value = serde_json::from_slice(&bytes).expect("JSON");
    unsupported["schema_version"] = json!(2);
    assert!(matches!(
        SnapshotManifest::from_json_slice(
            &serde_json::to_vec(&unsupported).expect("serialize invalid document")
        ),
        Err(ManifestError::UnsupportedSchemaVersion { found: 2 })
    ));

    let mut unknown: Value = serde_json::from_slice(&bytes).expect("JSON");
    unknown["untrusted_extension"] = json!(true);
    assert!(matches!(
        SnapshotManifest::from_json_slice(
            &serde_json::to_vec(&unknown).expect("serialize invalid document")
        ),
        Err(ManifestError::Json(_))
    ));
}

#[test]
fn manifest_rejects_unsafe_or_ambiguous_member_metadata() {
    let unsafe_path = SnapshotMember::try_new(
        "transcript",
        "../native-session.jsonl",
        12,
        digest('a'),
        SnapshotStorage::chunked(vec![object('b')]).expect("valid storage"),
    );
    assert!(matches!(
        unsafe_path,
        Err(ManifestError::InvalidRelativePath { .. })
    ));

    assert!(matches!(
        SnapshotStorage::chunked(Vec::new()),
        Err(ManifestError::EmptyChunkList)
    ));
    assert!(serde_json::from_str::<SnapshotStorage>(r#"{"kind":"chunked","chunks":[]}"#).is_err());

    let mismatched_object = SnapshotMember::try_new(
        "settings",
        "settings.json",
        12,
        digest('a'),
        SnapshotStorage::object(object('b')),
    );
    assert!(matches!(
        mismatched_object,
        Err(ManifestError::ObjectDigestMismatch)
    ));

    let mut draft = SnapshotManifestDraft {
        snapshot_id: "snap_duplicate".to_string(),
        machine_id: "machine-local".to_string(),
        created_at: Utc::now(),
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
        members: vec![
            SnapshotMember::try_new(
                "transcript",
                "same.jsonl",
                1,
                digest('a'),
                SnapshotStorage::object(object('a')),
            )
            .expect("member"),
            SnapshotMember::try_new(
                "metadata",
                "same.jsonl",
                1,
                digest('b'),
                SnapshotStorage::object(object('b')),
            )
            .expect("member"),
        ],
    };
    assert!(matches!(
        SnapshotManifest::try_new(draft.clone()),
        Err(ManifestError::DuplicateMemberPath(path)) if path == "same.jsonl"
    ));

    draft.members.pop();
    draft.previous_snapshot_id = Some(draft.snapshot_id.clone());
    assert!(matches!(
        SnapshotManifest::try_new(draft),
        Err(ManifestError::PreviousSnapshotSelfReference)
    ));
}

#[test]
fn hashes_and_object_ids_have_one_canonical_text_form() {
    let uppercase = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";
    let digest = Sha256Digest::from_str(uppercase).expect("valid uppercase input");
    assert_eq!(digest.to_string(), uppercase.to_ascii_lowercase());

    let object = ObjectId::from_str(&format!("sha256:{uppercase}")).expect("valid object id");
    assert_eq!(
        object.to_string(),
        format!("sha256:{}", uppercase.to_ascii_lowercase())
    );

    assert!(matches!(
        Sha256Digest::from_str("not-a-sha256"),
        Err(ManifestError::InvalidSha256)
    ));
    assert!(matches!(
        ObjectId::from_str(&format!("sha512:{uppercase}")),
        Err(ManifestError::InvalidObjectId)
    ));
}

#[test]
fn manifest_store_atomically_creates_once_and_never_overwrites() {
    let temp = tempfile::tempdir().expect("temporary vault");
    let snapshots_root = temp.path().join("snapshots");
    std::fs::create_dir(&snapshots_root).expect("snapshots root");
    let store = SnapshotManifestStore::open(&snapshots_root).expect("safe snapshots root");
    assert!(matches!(
        store.read(""),
        Err(ManifestError::InvalidSnapshotId {
            field: "snapshot_id"
        })
    ));
    let original = manifest();
    let original_bytes = original.to_json_bytes().expect("serialize");

    let path = store.write_new(&original).expect("first write");
    assert_eq!(
        path,
        snapshots_root.join("snap_01JTEST").join("manifest.json")
    );
    assert_eq!(
        std::fs::read(&path).expect("manifest bytes"),
        original_bytes
    );
    assert_eq!(store.read("snap_01JTEST").expect("read"), original);

    let mut changed_json: Value = serde_json::from_slice(&original_bytes).expect("JSON");
    changed_json["reason"] = json!("manual");
    let changed = SnapshotManifest::from_json_slice(
        &serde_json::to_vec(&changed_json).expect("changed JSON"),
    )
    .expect("changed manifest");
    assert!(matches!(
        store.write_new(&changed),
        Err(ManifestError::VaultIo(
            vault_io::Error::AtomicWriteConflict(_)
        ))
    ));
    assert_eq!(
        std::fs::read(path).expect("unchanged manifest"),
        original_bytes
    );
}
