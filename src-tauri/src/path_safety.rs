use std::fs::Metadata;
use std::path::Path;

use crate::error::AppResult;

pub use vault_io::path_safety::EntryKind;

pub fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    vault_io::path_safety::metadata_is_link_or_reparse(metadata)
}

pub fn validate_descendant(
    root: &Path,
    path: &Path,
    expected: EntryKind,
    allow_missing_leaf: bool,
    label: &str,
) -> AppResult<bool> {
    vault_io::path_safety::validate_descendant(root, path, expected, allow_missing_leaf, label)
        .map_err(Into::into)
}

pub fn validate_tree(root: &Path, path: &Path, label: &str) -> AppResult<()> {
    vault_io::path_safety::validate_tree(root, path, label).map_err(Into::into)
}

pub fn remove_path(root: &Path, path: &Path, expected: EntryKind, label: &str) -> AppResult<bool> {
    vault_io::path_safety::remove_path(root, path, expected, label).map_err(Into::into)
}
