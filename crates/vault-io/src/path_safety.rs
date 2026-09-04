use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

use crate::{Error as AppError, Result as AppResult};

/// Remove Windows verbatim path prefixes while preserving UNC semantics.
pub fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{}", rest);
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    s.to_string()
}

/// Validate a relative path supplied by an external manifest or archive.
pub fn checked_relative_path(raw: &str) -> AppResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Path("相对路径不能为空".into()));
    }
    if trimmed.contains('\0') {
        return Err(AppError::Path(format!("路径包含 NUL 字符: {raw}")));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(AppError::Path(format!("拒绝绝对路径: {raw}")));
    }

    let normalized = trimmed.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(AppError::Path(format!("拒绝 Windows 盘符路径: {raw}")));
    }

    let mut out = PathBuf::new();
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(AppError::Path(format!("拒绝目录穿越路径: {raw}")));
        }
        if segment.contains(':') {
            return Err(AppError::Path(format!("路径片段包含冒号: {raw}")));
        }
        if segment.chars().any(|c| c.is_control()) {
            return Err(AppError::Path(format!("路径包含控制字符: {raw}")));
        }
        out.push(segment);
    }

    if out.as_os_str().is_empty() {
        return Err(AppError::Path(format!("相对路径无有效片段: {raw}")));
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    FileOrDirectory,
}

pub fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn clean_path(path: &Path) -> PathBuf {
    PathBuf::from(strip_verbatim(&path.to_string_lossy()))
}

fn checked_relative(root: &Path, path: &Path, label: &str) -> AppResult<PathBuf> {
    let clean_root = clean_path(root);
    let clean_path = clean_path(path);
    let relative = clean_path.strip_prefix(&clean_root).map_err(|_| {
        AppError::Path(format!(
            "{label} 不在允许的根目录内: {}",
            path.to_string_lossy()
        ))
    })?;
    let checked = checked_relative_path(&relative.to_string_lossy())?;
    if checked.as_os_str().is_empty() {
        return Err(AppError::Path(format!(
            "{label} 不能指向根目录本身: {}",
            path.to_string_lossy()
        )));
    }
    Ok(checked)
}

fn validate_kind(
    path: &Path,
    metadata: &Metadata,
    expected: EntryKind,
    label: &str,
) -> AppResult<()> {
    let matches = match expected {
        EntryKind::File => metadata.is_file(),
        EntryKind::Directory => metadata.is_dir(),
        EntryKind::FileOrDirectory => metadata.is_file() || metadata.is_dir(),
    };
    if !matches {
        return Err(AppError::Path(format!(
            "{label} 类型不符合预期: {}",
            path.to_string_lossy()
        )));
    }
    Ok(())
}

/// Validate an existing or to-be-created descendant without following links or Windows reparse
/// points. Returns true when the leaf already exists.
pub fn validate_descendant(
    root: &Path,
    path: &Path,
    expected: EntryKind,
    allow_missing_leaf: bool,
    label: &str,
) -> AppResult<bool> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| {
        AppError::Path(format!(
            "{label} 根目录不可用 {}: {error}",
            root.to_string_lossy()
        ))
    })?;
    if metadata_is_link_or_reparse(&root_metadata) || !root_metadata.is_dir() {
        return Err(AppError::Path(format!(
            "{label} 根目录必须是普通目录且不能是链接或 junction: {}",
            root.to_string_lossy()
        )));
    }
    let canonical_root = root.canonicalize()?;
    let relative = checked_relative(root, path, label)?;
    let components = relative.components().count();
    let mut current = root.to_path_buf();
    let mut missing = false;

    for (index, component) in relative.components().enumerate() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if missing {
                    return Err(AppError::Path(format!(
                        "{label} 路径结构在缺失父目录后出现实体: {}",
                        current.to_string_lossy()
                    )));
                }
                if metadata_is_link_or_reparse(&metadata) {
                    return Err(AppError::Path(format!(
                        "{label} 路径包含符号链接或 junction，已拒绝: {}",
                        current.to_string_lossy()
                    )));
                }
                let is_leaf = index + 1 == components;
                if is_leaf {
                    validate_kind(&current, &metadata, expected, label)?;
                } else if !metadata.is_dir() {
                    return Err(AppError::Path(format!(
                        "{label} 的父路径不是目录: {}",
                        current.to_string_lossy()
                    )));
                }
                let canonical = current.canonicalize()?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(AppError::Path(format!(
                        "{label} 解析后逃出允许的根目录: {}",
                        current.to_string_lossy()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
            }
            Err(error) => return Err(error.into()),
        }
    }

    if missing && !allow_missing_leaf {
        return Err(AppError::NotFound(format!(
            "{label} 不存在: {}",
            path.to_string_lossy()
        )));
    }
    Ok(!missing)
}

/// Validate every entry in a directory tree. Walk errors and links are surfaced explicitly.
pub fn validate_tree(root: &Path, path: &Path, label: &str) -> AppResult<()> {
    validate_descendant(root, path, EntryKind::FileOrDirectory, false, label)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(());
    }
    let canonical_root = root.canonicalize()?;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|error| {
            AppError::Other(format!(
                "遍历 {label} 失败 {}: {error}",
                path.to_string_lossy()
            ))
        })?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(AppError::Path(format!(
                "{label} 内包含符号链接或 junction，已拒绝: {}",
                entry.path().to_string_lossy()
            )));
        }
        let canonical = entry.path().canonicalize()?;
        if !canonical.starts_with(&canonical_root) {
            return Err(AppError::Path(format!(
                "{label} 内条目解析后逃出允许的根目录: {}",
                entry.path().to_string_lossy()
            )));
        }
    }
    Ok(())
}

pub fn remove_path(root: &Path, path: &Path, expected: EntryKind, label: &str) -> AppResult<bool> {
    if !validate_descendant(root, path, expected, true, label)? {
        return Ok(false);
    }
    validate_tree(root, path, label)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_relative_path_normalizes_separators_and_dots() {
        assert_eq!(
            checked_relative_path(r"sessions\.\2026/file.jsonl").unwrap(),
            PathBuf::from("sessions").join("2026").join("file.jsonl")
        );
    }

    #[test]
    fn checked_relative_path_rejects_escape_and_absolute_paths() {
        assert!(checked_relative_path("../outside.jsonl").is_err());
        assert!(checked_relative_path("/absolute.jsonl").is_err());
        assert!(checked_relative_path(r"C:\absolute.jsonl").is_err());
    }

    #[test]
    fn strip_verbatim_preserves_unc_paths() {
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\file.jsonl"),
            r"\\server\share\file.jsonl"
        );
    }
}
