use thiserror::Error;

use crate::FullScanReason;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("registry database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("registry JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid registry record: {0}")]
    InvalidRecord(String),
    #[error("{kind} record not found: {id}")]
    MissingRecord { kind: &'static str, id: String },
    #[error("{kind} identity conflicts with existing record: {id}")]
    IdentityConflict { kind: &'static str, id: String },
    #[error("source cursor requires a full rebuild: {0:?}")]
    CursorRequiresRebuild(FullScanReason),
    #[error("registry schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("database has tables but is not a versioned AgentVault registry")]
    ConflictingSchema,
    #[error("AgentVault registry schema is inconsistent: {0}")]
    InconsistentSchema(String),
}

pub type RegistryResult<T> = Result<T, RegistryError>;

pub(crate) fn invalid_record(message: impl Into<String>) -> RegistryError {
    RegistryError::InvalidRecord(message.into())
}
