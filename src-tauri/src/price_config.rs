//! Bounded import and atomic no-replace export of user-selected price configurations.
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path};

use crate::error::{AppError, AppResult};

const MAX_PRICE_BYTES: usize = 1024 * 1024;

fn validate_path(path: &Path) -> AppResult<()> {
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        || path.components().any(|part| match part {
            Component::ParentDir => true,
            Component::Normal(name) => {
                name.to_string_lossy().contains([':', '/', '\\'])
                    || name.to_string_lossy().ends_with([' ', '.'])
            }
            _ => false,
        })
    {
        return Err(AppError::Path(
            "请选择不含路径跳转的绝对 .json 文件路径".into(),
        ));
    }
    for ancestor in path.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(AppError::Path(
                "单价配置目录不能是符号链接或 junction".into(),
            ));
        }
    }
    Ok(())
}

fn validate_json(json: &str) -> AppResult<()> {
    if json.len() > MAX_PRICE_BYTES {
        return Err(AppError::Other("单价配置超过 1 MiB 上限".into()));
    }
    let value: serde_json::Value =
        serde_json::from_str(json.strip_prefix('\u{feff}').unwrap_or(json))
            .map_err(|_| AppError::Other("单价配置不是有效 JSON".into()))?;
    let valid = value.as_object().is_some_and(|object| {
        object.len() == 2
            && object.get("version").and_then(|v| v.as_u64()) == Some(1)
            && object.get("prices").is_some_and(|v| v.is_object())
    });
    if !valid {
        return Err(AppError::Other(
            "单价配置必须包含 version: 1 和 prices 对象".into(),
        ));
    }
    Ok(())
}

pub fn save_usage_prices(path: &Path, json: &str) -> AppResult<()> {
    validate_json(json)?;
    validate_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(AppError::Path("目标已存在，请选择新的 JSON 文件名".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    crate::atomic_file::create_with_writer_if_absent(path, |file| {
        file.write_all(json.as_bytes())?;
        Ok(())
    })
}

pub fn read_usage_prices(path: &Path) -> AppResult<String> {
    validate_path(path)?;
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || crate::path_safety::metadata_is_link_or_reparse(&before) {
        return Err(AppError::Path(
            "单价配置必须是普通文件，不能是符号链接".into(),
        ));
    }
    if before.len() > MAX_PRICE_BYTES as u64 {
        return Err(AppError::Other("单价配置超过 1 MiB 上限".into()));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || crate::path_safety::metadata_is_link_or_reparse(&opened) {
        return Err(AppError::Path("单价配置必须是普通文件".into()));
    }
    // Limit the actual read too: a file can grow after metadata validation.
    let mut json = String::new();
    file.take(MAX_PRICE_BYTES as u64 + 1)
        .read_to_string(&mut json)?;
    validate_path(path)?;
    let after = fs::symlink_metadata(path)?;
    if crate::path_safety::metadata_is_link_or_reparse(&after)
        || !after.is_file()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(AppError::Path(
            "单价配置在读取过程中发生变化，请重试".into(),
        ));
    }
    validate_json(&json)?;
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONFIG: &str = r#"{"version":1,"prices":{}}"#;
    fn root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agentvault-price-config-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    #[test]
    fn price_config_roundtrip_and_no_replace() {
        let root = root();
        let path = root.join("prices.json");
        save_usage_prices(&path, CONFIG).unwrap();
        assert_eq!(read_usage_prices(&path).unwrap(), CONFIG);
        assert!(save_usage_prices(&path, CONFIG).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), CONFIG);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn price_config_preserves_utf8_bom() {
        let root = root();
        let path = root.join("bom.json");
        let json = format!("\u{feff}{CONFIG}");
        save_usage_prices(&path, &json).unwrap();
        assert_eq!(read_usage_prices(&path).unwrap(), json);
        assert_eq!(fs::read(&path).unwrap(), json.as_bytes());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn price_config_rejects_bad_paths_and_documents() {
        let root = root();
        for path in [
            Path::new("relative.json").to_path_buf(),
            root.join("prices.csv"),
            root.join("prices.json:ads.json"),
        ] {
            assert!(save_usage_prices(&path, CONFIG).is_err());
            assert!(read_usage_prices(&path).is_err());
        }
        for content in [
            "{}".to_string(),
            r#"{"version":2,"prices":{}}"#.into(),
            r#"{"version":1,"prices":[],"other":1}"#.into(),
            "x".repeat(MAX_PRICE_BYTES + 1),
        ] {
            let path = root.join("bad.json");
            assert!(save_usage_prices(&path, &content).is_err());
            fs::write(&path, content).unwrap();
            assert!(read_usage_prices(&path).is_err());
            fs::remove_file(path).unwrap();
        }
        let directory = root.join("directory.json");
        fs::create_dir(&directory).unwrap();
        assert!(read_usage_prices(&directory).is_err());
        assert!(save_usage_prices(&directory, CONFIG).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn price_config_concurrent_exports_create_only_one_file() {
        let root = root();
        let path = root.join("prices.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    save_usage_prices(&path, CONFIG).is_ok()
                })
            })
            .collect();
        let successes = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().then_some(()))
            .count();
        assert_eq!(successes, 1);
        assert_eq!(read_usage_prices(&path).unwrap(), CONFIG);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn price_config_rejects_file_and_parent_links() {
        let root = root();
        let original = root.join("original.json");
        fs::write(&original, CONFIG).unwrap();
        std::os::unix::fs::symlink(&original, root.join("link.json")).unwrap();
        assert!(read_usage_prices(&root.join("link.json")).is_err());
        assert!(save_usage_prices(&root.join("link.json"), CONFIG).is_err());
        fs::create_dir(root.join("actual")).unwrap();
        std::os::unix::fs::symlink(root.join("actual"), root.join("linked")).unwrap();
        assert!(save_usage_prices(&root.join("linked/new.json"), CONFIG).is_err());
        assert_eq!(fs::read_to_string(original).unwrap(), CONFIG);
        fs::remove_dir_all(root).unwrap();
    }
}
