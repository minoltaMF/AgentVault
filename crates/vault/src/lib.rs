//! Immutable snapshot metadata for AgentVault.
//!
//! This crate defines and validates snapshot manifest v1. It intentionally does not read
//! provider-native data, write content-addressed objects, or restore files. Manifest persistence
//! is write-once and rooted only at a directory explicitly supplied by the caller.

mod manifest;

pub use manifest::{
    CapturedResumePlan, ManifestError, ManifestResult, ObjectId, Sha256Digest, SnapshotConsistency,
    SnapshotManifest, SnapshotManifestDraft, SnapshotManifestStore, SnapshotMember,
    SnapshotStorage, SNAPSHOT_MANIFEST_SCHEMA_VERSION,
};
