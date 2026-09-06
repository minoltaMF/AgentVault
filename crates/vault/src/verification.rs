use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    ManifestError, ObjectId, ObjectStore, ObjectStoreError, Sha256Digest, SnapshotManifest,
    SnapshotManifestStore,
};

pub type SnapshotVerificationResult<T> = Result<T, SnapshotVerificationError>;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotVerificationError {
    #[error("snapshot manifest cannot be verified: {0}")]
    Manifest(#[from] ManifestError),
    #[error("snapshot object cannot be verified: {0}")]
    Object(#[from] ObjectStoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectFailureKind {
    Missing,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotVerificationIssue {
    ObjectUnavailable {
        member_path: String,
        object_id: ObjectId,
        failure: ObjectFailureKind,
    },
    MemberSizeMismatch {
        member_path: String,
        expected: u64,
        actual: u128,
    },
    MemberDigestMismatch {
        member_path: String,
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotVerificationReport {
    snapshot_id: String,
    verified: bool,
    member_count: usize,
    object_reference_count: usize,
    issues: Vec<SnapshotVerificationIssue>,
}

impl SnapshotVerificationReport {
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub const fn is_verified(&self) -> bool {
        self.verified
    }

    pub const fn member_count(&self) -> usize {
        self.member_count
    }

    pub const fn object_reference_count(&self) -> usize {
        self.object_reference_count
    }

    pub fn issues(&self) -> &[SnapshotVerificationIssue] {
        &self.issues
    }
}

/// Read-only verification over one immutable manifest and its referenced objects.
pub struct SnapshotVerifier<'a> {
    manifests: &'a SnapshotManifestStore,
    objects: &'a ObjectStore,
}

impl<'a> SnapshotVerifier<'a> {
    pub const fn new(manifests: &'a SnapshotManifestStore, objects: &'a ObjectStore) -> Self {
        Self { manifests, objects }
    }

    pub fn verify(
        &self,
        snapshot_id: &str,
    ) -> SnapshotVerificationResult<SnapshotVerificationReport> {
        let manifest = self.manifests.read(snapshot_id)?;
        self.verify_manifest(&manifest)
    }

    pub fn verify_manifest(
        &self,
        manifest: &SnapshotManifest,
    ) -> SnapshotVerificationResult<SnapshotVerificationReport> {
        let mut issues = Vec::new();
        let mut object_reference_count = 0;

        for member in manifest.members() {
            let mut member_complete = true;
            let mut actual_size = 0_u128;
            let mut hasher = Sha256::new();
            let objects = member
                .storage()
                .object_id()
                .map(std::slice::from_ref)
                .or_else(|| member.storage().chunks())
                .expect("validated snapshot storage has one representation");
            for object_id in objects {
                object_reference_count += 1;
                match self.objects.read(object_id) {
                    Ok(bytes) => {
                        actual_size += bytes.len() as u128;
                        hasher.update(&bytes);
                    }
                    Err(error) => match object_failure_kind(&error) {
                        Some(failure) => {
                            member_complete = false;
                            issues.push(SnapshotVerificationIssue::ObjectUnavailable {
                                member_path: member.relative_path().to_string(),
                                object_id: *object_id,
                                failure,
                            });
                        }
                        None => return Err(error.into()),
                    },
                }
            }
            if member_complete && actual_size != u128::from(member.size()) {
                issues.push(SnapshotVerificationIssue::MemberSizeMismatch {
                    member_path: member.relative_path().to_string(),
                    expected: member.size(),
                    actual: actual_size,
                });
            }
            if member_complete {
                let actual = Sha256Digest::from_bytes(hasher.finalize().into());
                if &actual != member.sha256() {
                    issues.push(SnapshotVerificationIssue::MemberDigestMismatch {
                        member_path: member.relative_path().to_string(),
                        expected: *member.sha256(),
                        actual,
                    });
                }
            }
        }

        Ok(SnapshotVerificationReport {
            snapshot_id: manifest.snapshot_id().to_string(),
            verified: issues.is_empty(),
            member_count: manifest.members().len(),
            object_reference_count,
            issues,
        })
    }
}

fn object_failure_kind(error: &ObjectStoreError) -> Option<ObjectFailureKind> {
    match error {
        ObjectStoreError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Some(ObjectFailureKind::Missing)
        }
        ObjectStoreError::VaultIo(vault_io::Error::NotFound(_)) => Some(ObjectFailureKind::Missing),
        ObjectStoreError::ObjectTooLarge { .. }
        | ObjectStoreError::EncodedObjectTooLarge { .. }
        | ObjectStoreError::InvalidEnvelope(_)
        | ObjectStoreError::UnsupportedFormatVersion(_)
        | ObjectStoreError::UnsupportedCodec(_)
        | ObjectStoreError::ObjectLengthMismatch { .. }
        | ObjectStoreError::ObjectDigestMismatch { .. }
        | ObjectStoreError::ObjectContentCollision(_) => Some(ObjectFailureKind::Corrupt),
        _ => None,
    }
}
