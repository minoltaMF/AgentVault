//! Immutable snapshot metadata for AgentVault.
//!
//! This crate defines immutable snapshot metadata, content-addressed object storage, and read-only
//! snapshot verification. It intentionally does not read provider-native data or restore files.
//! All persistence is rooted only at directories explicitly supplied by the caller.

mod manifest;
mod object_store;
mod verification;

pub use manifest::{
    CapturedResumePlan, ManifestError, ManifestResult, ObjectId, Sha256Digest, SnapshotConsistency,
    SnapshotManifest, SnapshotManifestDraft, SnapshotManifestStore, SnapshotMember,
    SnapshotStorage, SNAPSHOT_MANIFEST_SCHEMA_VERSION,
};
pub use object_store::{
    ObjectDisposition, ObjectStore, ObjectStoreError, ObjectStoreResult, StoredObject,
    MAX_OBJECT_BYTES,
};
pub use verification::{
    ObjectFailureKind, SnapshotVerificationError, SnapshotVerificationIssue,
    SnapshotVerificationReport, SnapshotVerificationResult, SnapshotVerifier,
};
