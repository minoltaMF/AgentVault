use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("path: {0}")]
    Path(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    AtomicWriteConflict(String),
    #[error("{0}")]
    AtomicWriteNotCommitted(String),
    #[error("{0}")]
    AtomicWriteCommitted(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn atomic_write_not_committed(&self) -> bool {
        matches!(
            self,
            Self::AtomicWriteConflict(_) | Self::AtomicWriteNotCommitted(_)
        )
    }

    pub fn retryable_atomic_write_conflict(&self) -> bool {
        matches!(self, Self::AtomicWriteConflict(_))
    }

    pub fn atomic_write_committed(&self) -> bool {
        matches!(self, Self::AtomicWriteCommitted(_))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
