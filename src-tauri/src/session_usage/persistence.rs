//! Versioned, disposable usage snapshots. The store lock serializes snapshot capture and CAS writes.
//! Native session files are never written. A rejected snapshot is preserved and caching stays in memory.
use super::cache::{self, SavedEntry, UsageCache};
use crate::{
    atomic_file,
    error::{AppError, AppResult},
    path_safety,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const VERSION: u32 = 1;
const PARSER_VERSION: u32 = 2;
const MAX_DISK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: String,
    version: u32,
    parser_version: u32,
    sha256: String,
    payload: String,
}
#[derive(Serialize, Deserialize)]
struct Payload {
    #[serde(deserialize_with = "bounded_entries")]
    entries: Vec<SavedEntry>,
}
fn bounded_entries<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<SavedEntry>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<SavedEntry>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 10000 usage entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element()? {
                if entries.len() == MAX_ENTRIES {
                    return Err(serde::de::Error::custom("too many usage entries"));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(Visitor)
}
fn failure(text: impl Into<String>) -> AppError {
    AppError::Other(text.into())
}
fn encode(entries: Vec<SavedEntry>) -> AppResult<Vec<u8>> {
    if entries.len() > MAX_ENTRIES {
        return Err(failure("用量缓存条数超过上限"));
    }
    let payload = serde_json::to_string(&Payload { entries })?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(failure("用量缓存内容超过上限"));
    }
    let bytes = serde_json::to_vec(&Envelope {
        format: "agentvault-usage-cache".into(),
        version: VERSION,
        parser_version: PARSER_VERSION,
        sha256: hex::encode(Sha256::digest(payload.as_bytes())),
        payload,
    })?;
    if bytes.len() as u64 > MAX_DISK_BYTES {
        return Err(failure("用量缓存文件超过上限"));
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> AppResult<Vec<SavedEntry>> {
    if bytes.len() as u64 > MAX_DISK_BYTES {
        return Err(failure("用量缓存文件超过上限"));
    }
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    if envelope.format != "agentvault-usage-cache"
        || envelope.version != VERSION
        || envelope.parser_version != PARSER_VERSION
    {
        return Err(failure("用量缓存格式或解析版本已变化，请使用安全重建缓存"));
    }
    if envelope.payload.len() > MAX_PAYLOAD_BYTES
        || envelope.sha256 != hex::encode(Sha256::digest(envelope.payload.as_bytes()))
    {
        return Err(failure("用量缓存大小或校验和无效"));
    }
    Ok(serde_json::from_str::<Payload>(&envelope.payload)?.entries)
}
// Check every existing ancestor, including Windows junctions, before using the atomic writer.
fn validate_path(path: &Path) -> AppResult<bool> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(failure("用量缓存需要不含父目录跳转的绝对路径"));
    }
    let parent = path.parent().ok_or_else(|| failure("用量缓存缺少父目录"))?;
    for ancestor in parent.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if path_safety::metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(failure("用量缓存父目录不是普通目录"));
        }
    }
    path_safety::validate_descendant(parent, path, path_safety::EntryKind::File, true, "用量缓存")
}
fn read_bounded(path: &Path) -> AppResult<Vec<u8>> {
    validate_path(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_DISK_BYTES {
        return Err(failure("用量缓存文件超过上限"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DISK_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DISK_BYTES {
        return Err(failure("用量缓存文件超过上限"));
    }
    validate_path(path)?;
    Ok(bytes)
}
struct Store {
    path: PathBuf,
    loaded: bool,
    writable: bool,
    expected: Option<atomic_file::FileFingerprint>,
    warning: Option<String>,
    backup_path: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize)]
pub struct UsageCacheStatus {
    pub state: String,
    pub disk_bytes: Option<u64>,
    pub memory_entries: usize,
    pub memory_bytes: usize,
    pub path: Option<String>,
    pub warning: Option<String>,
    pub backup_path: Option<String>,
}
impl Store {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: false,
            writable: true,
            expected: None,
            warning: None,
            backup_path: None,
        }
    }
    fn status(&self, cache: &Mutex<UsageCache>) -> UsageCacheStatus {
        let (memory_entries, memory_bytes) =
            cache.lock().unwrap_or_else(|e| e.into_inner()).stats();
        let checked = validate_path(&self.path).and_then(|exists| {
            if exists {
                let bytes = fs::symlink_metadata(&self.path)?.len();
                Ok(Some(bytes))
            } else {
                Ok(None)
            }
        });
        let (state, disk_bytes, warning) = match checked {
            Err(error) => ("degraded", None, Some(error.to_string())),
            Ok(Some(bytes)) if bytes > MAX_DISK_BYTES => (
                "degraded",
                Some(bytes),
                Some("用量缓存文件超过 64 MiB 上限，无法安全重建".into()),
            ),
            Ok(size) if !self.writable => ("degraded", size, self.warning.clone()),
            Ok(None) => ("missing", None, self.warning.clone()),
            Ok(size) => (
                if self.loaded { "ready" } else { "unchecked" },
                size,
                self.warning.clone(),
            ),
        };
        UsageCacheStatus {
            state: state.into(),
            disk_bytes,
            memory_entries,
            memory_bytes,
            path: Some(self.path.to_string_lossy().into_owned()),
            warning,
            backup_path: self
                .backup_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        }
    }
    fn rebuild(
        &mut self,
        cache: &Mutex<UsageCache>,
        after_backup: impl FnOnce(&Path),
    ) -> AppResult<()> {
        let result = (|| -> AppResult<()> {
            let exists = validate_path(&self.path)?;
            let expected = if exists {
                let bytes = read_bounded(&self.path)?;
                let expected = atomic_file::fingerprint_bytes(&bytes);
                let mut random = [0u8; 16];
                getrandom::getrandom(&mut random).map_err(|e| failure(e.to_string()))?;
                let backup = self.path.with_file_name(format!(
                    "{}.backup-{}",
                    self.path
                        .file_name()
                        .ok_or_else(|| failure("用量缓存缺少文件名"))?
                        .to_string_lossy(),
                    hex::encode(random)
                ));
                validate_path(&backup)?;
                // Retain the destination even if the atomic helper reports an ambiguous commit.
                self.backup_path = Some(backup.clone());
                atomic_file::create_with_writer_if_absent(&backup, |file| {
                    file.write_all(&bytes)?;
                    Ok(())
                })?;
                after_backup(&backup);
                if atomic_file::fingerprint_bytes(&read_bounded(&backup)?) != expected {
                    return Err(failure("用量缓存备份校验失败，已拒绝重建"));
                }
                Some(expected)
            } else {
                None
            };
            // Recheck ancestors immediately before the existing atomic CAS primitive.
            validate_path(&self.path)?;
            let bytes = encode(vec![])?;
            let next = atomic_file::fingerprint_bytes(&bytes);
            let writer = |file: &mut File| -> AppResult<()> {
                file.write_all(&bytes)?;
                Ok(())
            };
            match expected {
                Some(expected) => {
                    atomic_file::replace_with_writer_if_unchanged(&self.path, &expected, writer)?
                }
                None => atomic_file::create_with_writer_if_absent(&self.path, writer)?,
            }
            cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
            self.loaded = true;
            self.writable = true;
            self.expected = Some(next);
            self.warning = None;
            Ok(())
        })();
        if let Err(error) = result {
            self.writable = false;
            let message =
                format!("安全重建未完成；已生成的备份会保留，请检查缓存状态后重试：{error}");
            self.warning = Some(message.clone());
            return Err(failure(message));
        }
        Ok(())
    }
    fn warn(&mut self, error: AppError) {
        self.warning = Some(format!(
            "跨启动用量缓存不可用，本次仍可分析并复用内存结果：{error}"
        ));
    }
    fn load(&mut self, cache: &Mutex<UsageCache>) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let result = (|| -> AppResult<()> {
            if !validate_path(&self.path)? {
                return Ok(());
            }
            let bytes = read_bounded(&self.path)?;
            let entries = decode(&bytes)?;
            let expected = atomic_file::fingerprint_bytes(&bytes);
            // Validate that the pathname still contains the snapshot we read before accepting it.
            if atomic_file::fingerprint_bytes(&read_bounded(&self.path)?) != expected {
                return Err(failure("用量缓存加载期间发生变化"));
            }
            self.expected = Some(expected);
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .restore(entries);
            Ok(())
        })();
        if let Err(error) = result {
            self.writable = false;
            self.warn(error);
        }
    }
    fn save(&mut self, cache: &Mutex<UsageCache>) {
        self.load(cache);
        if !self.writable {
            return;
        }
        let result = (|| -> AppResult<()> {
            validate_path(&self.path)?;
            let entries = cache.lock().unwrap_or_else(|e| e.into_inner()).snapshot();
            let bytes = encode(entries)?;
            let next = atomic_file::fingerprint_bytes(&bytes);
            if self.expected.as_ref() == Some(&next) {
                return Ok(());
            }
            let writer = |file: &mut File| -> AppResult<()> {
                file.write_all(&bytes)?;
                Ok(())
            };
            match &self.expected {
                Some(expected) => {
                    atomic_file::replace_with_writer_if_unchanged(&self.path, expected, writer)?
                }
                None => atomic_file::create_with_writer_if_absent(&self.path, writer)?,
            }
            self.expected = Some(next);
            self.warning = None;
            Ok(())
        })();
        if let Err(error) = result {
            // Do not retry an ambiguous commit or overwrite a concurrent process's state.
            self.writable = false;
            self.warn(error);
        }
    }
}
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
pub(super) fn cache_status() -> AppResult<UsageCacheStatus> {
    if let Some(store) = STORE.get() {
        return Ok(store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(cache::shared()));
    }
    let (memory_entries, memory_bytes) = cache::shared()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .stats();
    Ok(UsageCacheStatus {
        state: "disabled".into(),
        disk_bytes: None,
        memory_entries,
        memory_bytes,
        path: None,
        warning: Some("当前进程未配置持久用量缓存".into()),
        backup_path: None,
    })
}
pub(super) fn rebuild_cache() -> AppResult<UsageCacheStatus> {
    let store = STORE
        .get()
        .ok_or_else(|| failure("当前进程未配置持久用量缓存"))?;
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    store.rebuild(cache::shared(), |_| {})?;
    Ok(store.status(cache::shared()))
}
/// Configure once during startup. Loading and saving happen only in usage worker threads.
pub fn configure_cache_path(path: PathBuf) {
    let _ = STORE.set(Mutex::new(Store::new(path)));
}
pub(super) fn load() -> Option<String> {
    let store = STORE.get()?;
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    store.load(cache::shared());
    store.warning.clone()
}
pub(super) fn save() -> Option<String> {
    let store = STORE.get()?;
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    store.save(cache::shared());
    store.warning.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_usage::{read_usage_cached, UsageSource};
    use std::sync::atomic::AtomicBool;
    fn cache() -> Mutex<UsageCache> {
        Mutex::new(UsageCache::new(10000, 16 * 1024 * 1024))
    }
    fn source(root: &Path) -> UsageSource {
        let path = root.join("projects/test/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, row("one")).unwrap();
        UsageSource {
            provider: "claude".into(),
            rollout_path: path.to_string_lossy().into_owned(),
            id: "test".into(),
            title: "current".into(),
            cwd: "".into(),
        }
    }
    fn row(id: &str) -> String {
        format!("{{\"type\":\"assistant\",\"requestId\":\"{id}\",\"message\":{{\"id\":\"{id}\",\"model\":\"synthetic\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":2}}}}}}\n")
    }
    #[test]
    fn dated_and_undated_buckets_survive_snapshot_roundtrip() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let source = source(&root);
        let dated = row("dated").replace(
            "\"type\":",
            "\"timestamp\":\"2026-10-12T00:00:00Z\",\"type\":",
        );
        fs::write(&source.rollout_path, format!("{dated}{}", row("undated"))).unwrap();
        let first = cache();
        let cancel = AtomicBool::new(false);
        let (original, _) =
            read_usage_cached(&source, &root, &cancel, false, &first, || {}).unwrap();
        let bytes = encode(first.lock().unwrap().snapshot()).unwrap();
        let second = cache();
        second.lock().unwrap().restore(decode(&bytes).unwrap());
        let (restored, hit) =
            read_usage_cached(&source, &root, &cancel, false, &second, || {}).unwrap();
        assert!(hit);
        assert_eq!(restored.daily, original.daily);
        assert_eq!(restored.undated_models, original.undated_models);
        assert_eq!(restored.daily[0].date, "2026-10-12");
        assert_eq!(
            restored.daily[0].models[0].tokens.input + restored.undated_models[0].tokens.input,
            restored.models[0].tokens.input
        );
        assert_eq!(
            restored.daily[0].models[0].tokens.output + restored.undated_models[0].tokens.output,
            restored.models[0].tokens.output
        );
    }

    #[test]
    fn pre_daily_parser_snapshot_is_rejected() {
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&encode(vec![]).unwrap()).unwrap();
        envelope["parser_version"] = serde_json::json!(1);
        assert!(decode(&serde_json::to_vec(&envelope).unwrap()).is_err());
    }

    #[test]
    fn rebuild_preserves_verified_backup_and_clears_memory() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("cache.json");
        let cache = cache();
        let source = source(&root);
        read_usage_cached(
            &source,
            &root,
            &AtomicBool::new(false),
            false,
            &cache,
            || {},
        )
        .unwrap();
        let mut store = Store::new(path.clone());
        assert_eq!(store.status(&cache).state, "missing");
        fs::write(&path, b"{broken").unwrap();
        assert_eq!(store.status(&cache).state, "unchecked");
        store.load(&cache);
        assert_eq!(store.status(&cache).state, "degraded");
        store.rebuild(&cache, |_| {}).unwrap();
        let backup = store.backup_path.clone().unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"{broken");
        assert_eq!(cache.lock().unwrap().stats(), (0, 0));
        assert!(decode(&fs::read(&path).unwrap()).unwrap().is_empty());
        assert_eq!(store.status(&cache).state, "ready");
        let valid = fs::read(&path).unwrap();
        store.rebuild(&cache, |_| {}).unwrap();
        assert_ne!(store.backup_path.as_ref(), Some(&backup));
        assert_eq!(fs::read(&backup).unwrap(), b"{broken");
        assert_eq!(
            fs::read(store.backup_path.as_ref().unwrap()).unwrap(),
            valid
        );
        let mut missing = Store::new(root.join("missing.json"));
        missing.rebuild(&cache, |_| {}).unwrap();
        assert!(missing.backup_path.is_none());
        assert_eq!(missing.status(&cache).state, "ready");
    }
    #[test]
    fn rebuild_rejects_changed_source_and_corrupted_backup() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("cache.json");
        fs::write(&path, b"original").unwrap();
        let mut store = Store::new(path.clone());
        assert!(store
            .rebuild(&cache(), |_| fs::write(&path, b"concurrent").unwrap())
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"concurrent");
        assert_eq!(
            fs::read(store.backup_path.as_ref().unwrap()).unwrap(),
            b"original"
        );
        assert!(store
            .rebuild(&cache(), |backup| fs::write(backup, b"bad backup").unwrap())
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"concurrent");
        assert_eq!(store.status(&cache()).state, "degraded");
    }
    #[test]
    fn rebuild_rejects_oversized_and_directory_without_backup() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("cache.json");
        File::create(&path)
            .unwrap()
            .set_len(MAX_DISK_BYTES + 1)
            .unwrap();
        let mut store = Store::new(path.clone());
        assert!(store
            .rebuild(&cache(), |_| panic!("must reject before backup"))
            .is_err());
        assert!(store.backup_path.is_none());
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_DISK_BYTES + 1);
        let mut directory = Store::new(root);
        assert!(directory.rebuild(&cache(), |_| {}).is_err());
        assert!(directory.backup_path.is_none());
    }
    #[test]
    fn restart_reuses_then_invalidates_and_force_refreshes() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let source = source(&root);
        let path = root.join("usage-cache-v1.json");
        let first = cache();
        let mut store = Store::new(path.clone());
        store.load(&first);
        let cancel = AtomicBool::new(false);
        assert!(
            !read_usage_cached(&source, &root, &cancel, false, &first, || {})
                .unwrap()
                .1
        );
        store.save(&first);
        assert!(store.warning.is_none());
        assert!(path.is_file());
        let second = cache();
        let mut restarted = Store::new(path);
        restarted.load(&second);
        assert!(restarted.warning.is_none());
        let (restored, hit) =
            read_usage_cached(&source, &root, &cancel, false, &second, || {}).unwrap();
        assert!(hit);
        assert!(restored.daily.is_empty());
        assert_eq!(restored.undated_models, restored.models);
        assert!(
            !read_usage_cached(&source, &root, &cancel, true, &second, || {})
                .unwrap()
                .1
        );
        fs::write(
            &source.rollout_path,
            format!("{}{}", row("one"), row("two")),
        )
        .unwrap();
        let (parsed, hit) =
            read_usage_cached(&source, &root, &cancel, false, &second, || {}).unwrap();
        assert!(!hit);
        assert_eq!(parsed.models[0].tokens.input, 20);
        fs::write(&source.rollout_path, "{").unwrap();
        assert!(read_usage_cached(&source, &root, &cancel, false, &second, || {}).is_err());
    }
    #[test]
    fn rejects_corruption_versions_and_oversized_input_without_overwrite() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap().join("cache.json");
        let valid = encode(vec![]).unwrap();
        for field in ["version", "parser_version"] {
            let mut envelope: serde_json::Value = serde_json::from_slice(&valid).unwrap();
            envelope[field] = 999.into();
            assert!(decode(&serde_json::to_vec(&envelope).unwrap()).is_err());
        }
        let mut envelope: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        envelope["sha256"] = "broken".into();
        assert!(decode(&serde_json::to_vec(&envelope).unwrap()).is_err());
        fs::write(&path, b"{corrupt").unwrap();
        let mut store = Store::new(path.clone());
        store.load(&cache());
        store.save(&cache());
        assert!(store.warning.is_some());
        assert_eq!(fs::read(&path).unwrap(), b"{corrupt");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_DISK_BYTES + 1).unwrap();
        assert!(read_bounded(&path).is_err());
        let root = fs::canonicalize(dir.path()).unwrap();
        let entry = SavedEntry {
            key: cache::Key::new(&source(&root), &root),
            fingerprint: (0, std::time::SystemTime::UNIX_EPOCH, "id".into()),
            parsed: Default::default(),
        };
        let payload = serde_json::to_string(&Payload {
            entries: vec![entry.clone(); MAX_ENTRIES],
        })
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Payload>(&payload)
                .unwrap()
                .entries
                .len(),
            MAX_ENTRIES
        );
        let oversized = serde_json::to_string(&Payload {
            entries: vec![entry; MAX_ENTRIES + 1],
        })
        .unwrap();
        assert!(serde_json::from_str::<Payload>(&oversized)
            .err()
            .unwrap()
            .to_string()
            .contains("too many usage entries"));
    }
    #[test]
    fn conflicts_and_unavailable_destination_preserve_external_data() {
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("cache.json");
        let mut store = Store::new(path.clone());
        let cache = cache();
        store.load(&cache);
        fs::write(&path, b"another writer").unwrap();
        store.save(&cache);
        assert!(store.warning.is_some());
        assert_eq!(fs::read(&path).unwrap(), b"another writer");
        let mut unavailable = Store::new(root.join("absent/cache.json"));
        unavailable.load(&cache);
        unavailable.save(&cache);
        assert!(unavailable.warning.is_some());
        assert!(!root.join("absent").exists());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_cache_and_ancestor() {
        use std::os::unix::fs::symlink;
        let dir = crate::readonly_source::test_support::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("target"), b"original").unwrap();
        symlink(root.join("target"), root.join("cache")).unwrap();
        let mut store = Store::new(root.join("cache"));
        assert!(store.rebuild(&cache(), |_| {}).is_err());
        assert!(store.backup_path.is_none());
        store.save(&cache());
        assert!(store.warning.is_some());
        assert_eq!(fs::read(root.join("target")).unwrap(), b"original");
        symlink(&root, root.join("alias")).unwrap();
        assert!(validate_path(&root.join("alias/new")).is_err());
    }
}
