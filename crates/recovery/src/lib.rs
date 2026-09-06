//! Deterministic, offline Recovery Capsule generation for AgentVault.

mod model;
mod render;

pub use model::{
    EvidenceQuality, EvidenceRef, RecoveryArtifactInput, RecoveryCapsule, RecoveryCapsuleDraft,
    RecoveryCapsuleError, RecoveryCapsuleResult, RecoveryCheckpointInput, RecoveryEventInput,
    RecoveryEventKind, RecoveryGitInput, RecoveryIdentityInput, RecoveryNoteInput,
};
pub use render::RECOVERY_CAPSULE_FORMAT;
