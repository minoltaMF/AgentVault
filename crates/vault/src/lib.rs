//! Immutable snapshot metadata for AgentVault.
//!
//! This crate defines immutable snapshot metadata and content-addressed object storage. It
//! intentionally does not read provider-native data or restore files. All persistence is rooted
//! only at directories explicitly supplied by the caller.

mod manifest;
mod object_store;

pub use manifest::{
    CapturedResumePlan, ManifestError, ManifestResult, ObjectId, Sha256Digest, SnapshotConsistency,
    SnapshotManifest, SnapshotManifestDraft, SnapshotManifestStore, SnapshotMember,
    SnapshotStorage, SNAPSHOT_MANIFEST_SCHEMA_VERSION,
};
pub use object_store::{
    ObjectDisposition, ObjectStore, ObjectStoreError, ObjectStoreResult, StoredObject,
    MAX_OBJECT_BYTES,
};
