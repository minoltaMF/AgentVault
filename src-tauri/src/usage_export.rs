//! Save a generated usage report without replacing any existing file.
use std::fs;
use std::io::Write;
use std::path::{Component, Path};

use crate::error::{AppError, AppResult};

const MAX_CSV_BYTES: usize = 32 * 1024 * 1024;

pub fn save_usage_csv(out_path: &Path, csv: &str) -> AppResult<()> {
    if !out_path.is_absolute()
        || !out_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        || out_path.components().any(|part| match part {
            Component::ParentDir => true,
            Component::Normal(name) => {
                name.to_string_lossy().contains([':', '/', '\\'])
                    || name.to_string_lossy().ends_with([' ', '.'])
            }
            _ => false,
        })
    {
        return Err(AppError::Path(
            "请选择不含路径跳转的绝对 .csv 文件路径".into(),
        ));
    }
    let body = csv.strip_prefix('\u{feff}').unwrap_or(csv);
    if csv.len() > MAX_CSV_BYTES
        || csv.contains('\0')
        || !(body.starts_with("record_type,") || body.starts_with("\"record_type\","))
        || !body.contains('\n')
    {
        return Err(AppError::Other(
            "用量 CSV 内容无效或超过 32 MiB 保存上限".into(),
        ));
    }
    // Reject existing entries, including dangling links, before creating the staged file.
    match fs::symlink_metadata(out_path) {
        Ok(_) => return Err(AppError::Path("目标已存在，请选择新的 CSV 文件名".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for ancestor in out_path.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(AppError::Path(
                "CSV 保存目录不能是符号链接或 junction".into(),
            ));
        }
    }
    // The atomic no-replace commit also rejects a destination created after validation.
    crate::atomic_file::create_with_writer_if_absent(out_path, |file| {
        file.write_all(csv.as_bytes())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    const REPORT: &str = "\u{feff}record_type,value\r\nsummary,42\r\n";

    fn root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agentvault-usage-export-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }

    #[test]
    fn usage_export_preserves_bytes_and_existing_targets() {
        let root = root();
        let path = root.join("report.csv");
        save_usage_csv(&path, REPORT).unwrap();
        assert_eq!(fs::read(&path).unwrap(), REPORT.as_bytes());
        assert!(save_usage_csv(&path, "record_type,value\nsummary,99\n").is_err());
        assert_eq!(fs::read(&path).unwrap(), REPORT.as_bytes());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usage_export_rejects_invalid_paths_and_content() {
        let root = root();
        // PathBuf::join normalizes parent components for Windows verbatim paths;
        // preserve the raw user input so this actually exercises traversal rejection.
        let mut traversal = root.as_os_str().to_os_string();
        traversal.push(format!(
            "{}..{}escape.csv",
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ));
        for path in [
            Path::new("relative.csv").to_path_buf(),
            root.join("session.jsonl"),
            std::path::PathBuf::from(traversal),
            root.join("report.csv:stream.csv"),
        ] {
            assert!(
                save_usage_csv(&path, REPORT).is_err(),
                "unexpected accepted path: {path:?}"
            );
        }
        for csv in [
            String::new(),
            "not,csv\n".into(),
            "record_type,value\n\0".into(),
            format!("record_type,value\n{}", "x".repeat(MAX_CSV_BYTES)),
        ] {
            assert!(save_usage_csv(&root.join("report.csv"), &csv).is_err());
        }
        fs::create_dir(root.join("directory.csv")).unwrap();
        assert!(save_usage_csv(&root.join("directory.csv"), REPORT).is_err());
        assert!(!root.join("report.csv").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usage_export_concurrent_saves_commit_exactly_once() {
        let root = root();
        let path = root.join("report.csv");
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = ["record_type,value\na,1\n", "record_type,value\nb,2\n"]
            .into_iter()
            .map(|csv| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    save_usage_csv(&path, csv).is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .filter_map(|handle| handle.join().unwrap().then_some(()))
                .count(),
            1
        );
        let saved = fs::read_to_string(path).unwrap();
        assert!(saved == "record_type,value\na,1\n" || saved == "record_type,value\nb,2\n");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn usage_export_rejects_dangling_and_directory_links() {
        let root = root();
        std::os::unix::fs::symlink(root.join("missing"), root.join("report.csv")).unwrap();
        assert!(save_usage_csv(&root.join("report.csv"), REPORT).is_err());
        fs::create_dir(root.join("actual")).unwrap();
        std::os::unix::fs::symlink(root.join("actual"), root.join("linked")).unwrap();
        assert!(save_usage_csv(&root.join("linked/report.csv"), REPORT).is_err());
        assert_eq!(fs::read_dir(root.join("actual")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
