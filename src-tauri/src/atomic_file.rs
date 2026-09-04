use std::fs::File;
use std::path::Path;

use crate::error::{AppError, AppResult};
use vault_io::atomic;
use vault_io::Error as VaultIoError;

pub use vault_io::atomic::FileFingerprint;

fn writer_error(error: AppError) -> VaultIoError {
    match error {
        AppError::AtomicWriteConflict(message) => VaultIoError::AtomicWriteConflict(message),
        AppError::AtomicWriteNotCommitted(message) => {
            VaultIoError::AtomicWriteNotCommitted(message)
        }
        AppError::AtomicWriteCommitted(message) => VaultIoError::AtomicWriteCommitted(message),
        other => VaultIoError::Other(other.to_string()),
    }
}

pub fn fingerprint(path: &Path) -> AppResult<FileFingerprint> {
    atomic::fingerprint(path).map_err(Into::into)
}

pub(crate) fn fingerprint_bytes(bytes: &[u8]) -> FileFingerprint {
    atomic::fingerprint_bytes(bytes)
}

pub fn replace_with_writer_if_unchanged(
    path: &Path,
    expected: &FileFingerprint,
    writer: impl FnOnce(&mut File) -> AppResult<()>,
) -> AppResult<()> {
    atomic::replace_with_writer_if_unchanged(path, expected, |file| {
        writer(file).map_err(writer_error)
    })
    .map_err(Into::into)
}

pub fn create_with_writer_if_absent(
    path: &Path,
    writer: impl FnOnce(&mut File) -> AppResult<()>,
) -> AppResult<()> {
    atomic::create_with_writer_if_absent(path, |file| writer(file).map_err(writer_error))
        .map_err(Into::into)
}

pub fn overwrite_with_writer(
    path: &Path,
    writer: impl FnOnce(&mut File) -> AppResult<()>,
) -> AppResult<()> {
    atomic::overwrite_with_writer(path, |file| writer(file).map_err(writer_error))
        .map_err(Into::into)
}

pub(crate) fn move_file_if_absent(source: &Path, destination: &Path) -> AppResult<()> {
    atomic::move_file_if_absent(source, destination).map_err(Into::into)
}

pub(crate) fn remove_file_if_unchanged(
    path: &Path,
    expected: &FileFingerprint,
    label: &str,
) -> AppResult<()> {
    atomic::remove_file_if_unchanged(path, expected, label).map_err(Into::into)
}

pub(crate) fn remove_staged_file_if_unchanged(
    path: &Path,
    expected: &FileFingerprint,
    label: &str,
) -> AppResult<()> {
    atomic::remove_staged_file_if_unchanged(path, expected, label).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn writer_errors_keep_atomic_commit_classification() -> AppResult<()> {
        let root = std::env::temp_dir().join(format!(
            "agentvault-atomic-adapter-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&root)?;

        let conflict_path = root.join("conflict.json");
        let conflict = create_with_writer_if_absent(&conflict_path, |_| {
            Err(AppError::AtomicWriteConflict("concurrent writer".into()))
        })
        .expect_err("writer conflict must be returned");
        assert!(matches!(conflict, AppError::AtomicWriteConflict(_)));
        assert!(!conflict_path.exists());

        let failed_path = root.join("failed.json");
        let failed = create_with_writer_if_absent(&failed_path, |_| {
            Err(AppError::Io(std::io::Error::other("writer failed")))
        })
        .expect_err("writer failure must be returned as not committed");
        assert!(matches!(failed, AppError::AtomicWriteNotCommitted(_)));
        assert!(!failed_path.exists());

        fs::remove_dir_all(root).ok();
        Ok(())
    }
}
