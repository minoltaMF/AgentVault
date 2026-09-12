//! Durable pre-images of the Codex deletion footprint. This is deliberately separate from
//! portable session export: raw metadata, duplicate rollouts and complete SQLite stores matter.
//! Recovery materializes into a NEW directory only; it never replaces a live native database.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::path_safety::EntryKind;
use crate::{atomic_file, path_safety, paths};

const DATABASES: &[&str] = &[
    "state_5.sqlite",
    "logs_2.sqlite",
    "thread_history_1.sqlite",
    "sqlite/codex-dev.db",
    "sqlite/codex-thread-summaries-dev.db",
];
const METADATA: &[&str] = &[
    "session_index.jsonl",
    "session_family.json",
    "archive_ledger.json",
    ".codex-global-state.json",
    "history.jsonl",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Content {
    size: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    path: String,
    sqlite: bool,
    // Absence is part of the pre-image, not a silently skipped artifact.
    content: Option<Content>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    source_root: String,
    ids: Vec<String>,
    members: Vec<Member>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotReport {
    pub snapshot_path: String,
    pub verified: bool,
    pub files: usize,
}

#[derive(Debug, Serialize)]
pub struct SnapshotSummary {
    pub snapshot_path: String,
    pub name: String,
    pub created_at: Option<String>,
    pub source_root: Option<String>,
    pub session_ids: Vec<String>,
    pub files: usize,
    /// Declared payload bytes, not measured disk allocation or a hash verification.
    pub total_bytes: u64,
    pub status: &'static str,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotMember {
    pub path: String,
    pub sqlite: bool,
    pub present: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotDetail {
    #[serde(flatten)]
    pub summary: SnapshotSummary,
    pub members: Vec<SnapshotMember>,
}

// Check ancestors before canonicalizing: resolving first would hide a symlink/reparse hop.
fn checked_ui_root(backup_dir: &str) -> AppResult<PathBuf> {
    let root = backup_root(Some(backup_dir));
    if !root.is_absolute()
        || root.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(AppError::Path("备份目录必须是无路径穿越的绝对路径".into()));
    }
    let root = root.join("codex-delete-snapshots");
    for ancestor in root.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if !meta.is_dir() || path_safety::metadata_is_link_or_reparse(&meta) => {
                return Err(AppError::Path(
                    "快照路径必须经过普通目录，不能包含链接或重解析点".into(),
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(root)
}

fn checked_ui_snapshot(backup_dir: &str, snapshot_path: &str) -> AppResult<PathBuf> {
    let root = checked_ui_root(backup_dir)?;
    let path = PathBuf::from(snapshot_path);
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path.parent().map(normalized_absolute) != Some(normalized_absolute(&root))
    {
        return Err(AppError::Path(
            "只能访问当前备份目录中的直接子快照目录".into(),
        ));
    }
    path_safety::validate_descendant(&root, &path, EntryKind::Directory, false, "删除快照")?;
    Ok(path)
}

fn inspect_path(path: &Path) -> SnapshotDetail {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let created_at = name
        .strip_prefix("delete-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|stamp| chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").ok())
        .map(|stamp| stamp.and_utc().to_rfc3339());
    let mut detail = SnapshotDetail {
        summary: SnapshotSummary {
            snapshot_path: path.to_string_lossy().into_owned(),
            name,
            created_at,
            source_root: None,
            session_ids: vec![],
            files: 0,
            total_bytes: 0,
            status: "unreadable",
            error: None,
        },
        members: vec![],
    };
    let manifest = match load_manifest(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            detail.summary.status = if path.join("manifest.json").try_exists().ok() == Some(false) {
                "incomplete"
            } else {
                "unreadable"
            };
            detail.summary.error = Some(error.to_string());
            return detail;
        }
    };
    detail.summary.source_root = Some(manifest.source_root);
    detail.summary.session_ids = manifest.ids;
    detail.summary.status = "unverified";
    for member in manifest.members {
        let size = member.content.as_ref().map(|content| content.size);
        if let Some(size) = size {
            detail.summary.files += 1;
            match detail.summary.total_bytes.checked_add(size) {
                Some(total) => detail.summary.total_bytes = total,
                None => {
                    detail.summary.status = "unreadable";
                    detail.summary.error = Some("manifest 声明大小溢出".into());
                }
            }
        }
        let result = path_safety::validate_descendant(
            path,
            &path.join("files").join(&member.path),
            EntryKind::File,
            true,
            "快照内容",
        );
        let present = match result {
            Ok(present) => {
                if present != size.is_some() && detail.summary.status != "unreadable" {
                    detail.summary.status = "incomplete";
                    detail.summary.error = Some(format!("快照成员存在状态不一致: {}", member.path));
                }
                present
            }
            Err(error) => {
                detail.summary.status = "unreadable";
                detail.summary.error = Some(error.to_string());
                false
            }
        };
        detail.members.push(SnapshotMember {
            path: member.path,
            sqlite: member.sqlite,
            present,
            size,
        });
    }
    detail
}

pub fn list_delete_snapshots(backup_dir: &str) -> AppResult<Vec<SnapshotSummary>> {
    let root = checked_ui_root(backup_dir)?;
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry?;
        // Never traverse links. An individual unreadable directory remains visible.
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir() && !path_safety::metadata_is_link_or_reparse(&meta) {
            summaries.push(inspect_path(&entry.path()).summary);
        }
    }
    summaries.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(summaries)
}

pub fn inspect_delete_snapshot(backup_dir: &str, snapshot_path: &str) -> AppResult<SnapshotDetail> {
    Ok(inspect_path(&checked_ui_snapshot(
        backup_dir,
        snapshot_path,
    )?))
}

pub fn verify_delete_snapshot(backup_dir: &str, snapshot_path: &str) -> AppResult<SnapshotReport> {
    verify(&checked_ui_snapshot(backup_dir, snapshot_path)?)
}

pub fn restore_delete_snapshot(
    backup_dir: &str,
    snapshot_path: &str,
    output: &str,
) -> AppResult<SnapshotReport> {
    restore_to_new_dir(
        &checked_ui_snapshot(backup_dir, snapshot_path)?,
        Path::new(output),
    )
}

struct Observation {
    path: PathBuf,
    content: Option<Content>,
}

pub(crate) struct Preparation {
    source: PathBuf,
    pub(crate) path: PathBuf,
    members: Vec<Member>,
    observed: Vec<Observation>,
}

pub(crate) struct VerifiedSnapshot {
    pub(crate) path: PathBuf,
    source: PathBuf,
    observed: Vec<Observation>,
}

fn unique_name() -> AppResult<String> {
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| AppError::Other(error.to_string()))?;
    Ok(format!(
        "delete-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        hex::encode(random)
    ))
}

pub(crate) fn backup_root(configured: Option<&str>) -> PathBuf {
    if let Some(path) = configured.filter(|path| !path.trim().is_empty()) {
        return PathBuf::from(path);
    }
    #[cfg(test)]
    {
        TEST_BACKUP_ROOT.with(|root| root.0.clone())
    }
    #[cfg(not(test))]
    {
        paths::default_backup_dir()
    }
}

// Tests still run the real capture/verify pipeline, but may never write into a user's backup root.
#[cfg(test)]
struct TestBackupRoot(PathBuf);
#[cfg(test)]
impl Drop for TestBackupRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[cfg(test)]
thread_local! {
    static TEST_BACKUP_ROOT: TestBackupRoot = TestBackupRoot(std::env::temp_dir().join(unique_name().expect("test random")));
}

fn digest(path: &Path) -> AppResult<Content> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file, &mut hasher)?;
    Ok(Content {
        size,
        sha256: hex::encode(hasher.finalize()),
    })
}

fn observe(root: &Path, path: &Path) -> AppResult<Option<Content>> {
    if path_safety::validate_descendant(root, path, EntryKind::File, true, "删除快照源")? {
        Ok(Some(digest(path)?))
    } else {
        Ok(None)
    }
}

fn validate_observations(root: &Path, observations: &[Observation]) -> AppResult<()> {
    for observation in observations {
        if observe(root, &observation.path)? != observation.content {
            return Err(AppError::Other(format!(
                "删除快照期间源数据变化，已拒绝删除: {}",
                observation.path.display()
            )));
        }
    }
    Ok(())
}

// Resolve a possibly missing output root through its nearest existing ancestor, then validate
// each descendant before creating it. In particular, no backup may be placed inside Codex home.
fn resolved_destination(path: &Path) -> AppResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = absolute.as_path();
    while !ancestor.try_exists()? {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| AppError::Path("输出目录无有效父目录".into()))?;
    }
    let resolved = ancestor.canonicalize()?;
    if !resolved.is_dir() {
        return Err(AppError::Path("输出目录的父路径不是目录".into()));
    }
    if ancestor == absolute {
        return Ok(resolved);
    }
    let relative = absolute
        .strip_prefix(ancestor)
        .map_err(|error| AppError::Path(error.to_string()))?;
    Ok(resolved.join(paths::checked_relative_path(&relative.to_string_lossy())?))
}

fn safe_create_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            safe_create_dir(parent)?;
        }
        if !path_safety::validate_descendant(parent, path, EntryKind::Directory, true, "快照目录")?
        {
            fs::create_dir(path)?;
            sync_directory(parent)?;
        }
    }
    Ok(())
}

// Match vault-io's directory durability contract: Unix fsyncs directories; Windows atomic
// publication uses its native write-through primitive. Do not claim power-loss hardware tests.
fn sync_directory(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn normalized_absolute(path: &Path) -> PathBuf {
    let path = paths::strip_verbatim(&path.to_string_lossy());
    #[cfg(windows)]
    let path = path.to_lowercase();
    PathBuf::from(path)
}

fn within(path: &Path, root: &Path) -> bool {
    normalized_absolute(path).starts_with(normalized_absolute(root))
}

fn copy_new(source: &Path, destination: &Path) -> AppResult<()> {
    if let Some(parent) = destination.parent() {
        safe_create_dir(parent)?;
    }
    atomic_file::create_with_writer_if_absent(destination, |file| {
        std::io::copy(&mut File::open(source)?, file)?;
        Ok(())
    })
}

fn check_database(path: &Path) -> AppResult<()> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = connection.prepare("PRAGMA integrity_check")?;
    let checks = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if checks != ["ok"] {
        return Err(AppError::Other(format!(
            "快照 SQLite 完整性校验失败: {}",
            path.display()
        )));
    }
    Ok(())
}

impl Preparation {
    pub(crate) fn new(source: &Path, backup_root: &Path) -> AppResult<Self> {
        let source = source.canonicalize()?;
        let root = resolved_destination(&backup_root.join("codex-delete-snapshots"))?;
        if within(&root, &source) {
            return Err(AppError::Path(
                "删除快照目录不能位于 Codex 数据目录内".into(),
            ));
        }
        safe_create_dir(&root)?;
        let path = root.join(unique_name()?);
        fs::create_dir(&path)?;
        sync_directory(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        let mut preparation = Self {
            source,
            path,
            members: vec![],
            observed: vec![],
        };
        // Record all fixed sources BEFORE copying the first one, including WAL and rollback
        // journals. Reading staged copies avoids SQLite creating a source -shm on failed capture.
        for relative in METADATA.iter().chain(DATABASES) {
            let path = preparation.source.join(relative);
            let content = observe(&preparation.source, &path)?;
            if *relative == "state_5.sqlite" && content.is_none() {
                return Err(AppError::NotFound("删除前快照缺少 state_5.sqlite".into()));
            }
            preparation.observed.push(Observation { path, content });
            if DATABASES.contains(relative) {
                for suffix in ["-wal", "-journal"] {
                    let path = preparation.source.join(format!("{relative}{suffix}"));
                    let content = observe(&preparation.source, &path)?;
                    preparation.observed.push(Observation { path, content });
                }
            }
        }
        for relative in METADATA {
            preparation.capture_file(relative, false)?;
        }
        for relative in DATABASES {
            preparation.capture_database(relative)?;
        }
        validate_observations(&preparation.source, &preparation.observed)?;
        Ok(preparation)
    }

    pub(crate) fn files_root(&self) -> PathBuf {
        self.path.join("files")
    }

    fn capture_file(&mut self, relative: &str, sqlite: bool) -> AppResult<()> {
        if self.members.iter().any(|member| member.path == relative) {
            return Ok(());
        }
        let path = self.source.join(paths::checked_relative_path(relative)?);
        let content = observe(&self.source, &path)?;
        if !self
            .observed
            .iter()
            .any(|observation| observation.path == path)
        {
            self.observed.push(Observation {
                path: path.clone(),
                content: content.clone(),
            });
        }
        if let Some(expected) = content.as_ref() {
            let destination = self.files_root().join(relative);
            copy_new(&path, &destination)?;
            if &digest(&destination)? != expected {
                return Err(AppError::Other("删除快照复制后摘要不一致".into()));
            }
        }
        self.members.push(Member {
            path: relative.into(),
            sqlite,
            content,
        });
        Ok(())
    }

    fn capture_database(&mut self, relative: &str) -> AppResult<()> {
        let path = self.source.join(relative);
        if observe(&self.source, &path)?.is_none() {
            self.members.push(Member {
                path: relative.into(),
                sqlite: true,
                content: None,
            });
            return Ok(());
        }
        let staging = self.path.join("sqlite-staging");
        safe_create_dir(&staging)?;
        let staged = staging.join("source.sqlite");
        copy_new(&path, &staged)?;
        for suffix in ["-wal", "-journal"] {
            let sidecar = self.source.join(format!("{relative}{suffix}"));
            if observe(&self.source, &sidecar)?.is_some() {
                copy_new(&sidecar, &staging.join(format!("source.sqlite{suffix}")))?;
            }
        }
        let destination = self.files_root().join(relative);
        safe_create_dir(destination.parent().expect("database parent"))?;
        let native_output = staging.join("native.sqlite");
        {
            let input = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut output = Connection::open(&native_output)?;
            {
                let backup = Backup::new(&input, &mut output)?;
                loop {
                    match backup.step(256)? {
                        StepResult::Done => break,
                        StepResult::More => {}
                        _ => {
                            return Err(AppError::Other(
                                "快照 SQLite backup 无法取得锁，已拒绝删除".into(),
                            ))
                        }
                    }
                }
            }
            output.pragma_update(None, "journal_mode", "DELETE")?;
        }
        // Publish the closed native backup with the same fsync/create-if-absent primitive as
        // raw files (including Windows write-through rename), before publishing the manifest.
        copy_new(&native_output, &destination)?;
        check_database(&destination)?;
        self.members.push(Member {
            path: relative.into(),
            sqlite: true,
            content: Some(digest(&destination)?),
        });
        // Staging is exclusively owned by this capture, below its unique snapshot directory.
        path_safety::remove_path(
            &self.path,
            &staging,
            EntryKind::Directory,
            "SQLite 快照临时目录",
        )?;
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        ids: &[String],
        rollouts: &[PathBuf],
    ) -> AppResult<VerifiedSnapshot> {
        for path in rollouts {
            let canonical = path.canonicalize()?;
            let relative = canonical
                .strip_prefix(&self.source)
                .map_err(|error| AppError::Path(error.to_string()))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| AppError::Path("快照路径不是 UTF-8".into()))?
                .replace('\\', "/");
            self.capture_file(&relative, false)?;
        }
        validate_observations(&self.source, &self.observed)?;
        let manifest = Manifest {
            version: 1,
            source_root: self.source.to_string_lossy().into_owned(),
            ids: ids.to_vec(),
            members: self.members,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        atomic_file::create_with_writer_if_absent(&self.path.join("manifest.json"), |file| {
            file.write_all(&bytes)?;
            Ok(())
        })?;
        #[cfg(test)]
        inject_fault(&self.path, &self.source)?;
        // A durable write is not sufficient: reopen the manifest and every payload from disk.
        verify(&self.path)?;
        validate_observations(&self.source, &self.observed)?;
        Ok(VerifiedSnapshot {
            path: self.path,
            source: self.source,
            observed: self.observed,
        })
    }
}

impl VerifiedSnapshot {
    pub(crate) fn expected_file(
        &self,
        path: &Path,
    ) -> AppResult<Option<atomic_file::FileFingerprint>> {
        // Snapshot observations use canonical paths; Windows callers may still use an
        // 8.3 ancestor alias. Resolve existing ancestors for absent metadata as well.
        // Reject links before resolving so canonicalization cannot hide a reparse hop.
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(meta) if path_safety::metadata_is_link_or_reparse(&meta) => {
                    return Err(AppError::Path("删除快照源不能包含链接或重解析点".into()));
                }
                Ok(meta) => {
                    // Validate descendants through the selected source, not OS-level aliases
                    // above it (for example macOS /var -> /private/var).
                    if meta.is_dir() && ancestor.canonicalize()? == self.source {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let resolved = if path.try_exists()? {
            path.canonicalize()?
        } else {
            let parent = path
                .parent()
                .ok_or_else(|| AppError::Path("删除快照源缺少父目录".into()))?;
            let name = path
                .file_name()
                .ok_or_else(|| AppError::Path("删除快照源缺少文件名".into()))?;
            resolved_destination(parent)?.join(name)
        };
        let normalized = normalized_absolute(&resolved);
        let original = self
            .observed
            .iter()
            .find(|observation| normalized_absolute(&observation.path) == normalized)
            .ok_or_else(|| AppError::Path("文件未纳入删除快照".into()))?;
        let relative = original
            .path
            .strip_prefix(&self.source)
            .map_err(|error| AppError::Path(error.to_string()))?;
        if let Some(content) = &original.content {
            let stored = self.path.join("files").join(relative);
            if digest(&stored)? != *content {
                return Err(AppError::Other("删除快照预图已变化".into()));
            }
            Ok(Some(atomic_file::fingerprint(&stored)?))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn ensure_source_unchanged(&self) -> AppResult<()> {
        validate_observations(&self.source, &self.observed)
    }

    pub(crate) fn ensure_after_sqlite_open(&self) -> AppResult<()> {
        for observation in &self.observed {
            let current = observe(&self.source, &observation.path)?;
            // Opening a previously checkpointed WAL database can create an empty WAL. It holds
            // no transaction data; every pre-existing/nonempty WAL must still match exactly.
            let newly_empty_wal = observation.content.is_none()
                && current.as_ref().is_some_and(|content| content.size == 0)
                && DATABASES.iter().any(|database| {
                    observation.path == self.source.join(format!("{database}-wal"))
                });
            if current != observation.content && !newly_empty_wal {
                return Err(AppError::AtomicWriteConflict(format!(
                    "删除快照后源数据发生变化: {}",
                    observation.path.display()
                )));
            }
        }
        Ok(())
    }
}

fn load_manifest(snapshot: &Path) -> AppResult<Manifest> {
    let metadata = fs::symlink_metadata(snapshot)?;
    if !metadata.is_dir() || path_safety::metadata_is_link_or_reparse(&metadata) {
        return Err(AppError::Path("快照必须是普通目录".into()));
    }
    let manifest_path = snapshot.join("manifest.json");
    path_safety::validate_descendant(
        snapshot,
        &manifest_path,
        EntryKind::File,
        false,
        "快照 manifest",
    )?;
    // Bound untrusted manifest memory use. Payloads themselves are streamed.
    let mut bytes = Vec::new();
    File::open(&manifest_path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(AppError::Other("快照 manifest 超过 16 MiB".into()));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.version != 1
        || manifest.ids.is_empty()
        || !Path::new(&manifest.source_root).is_absolute()
    {
        return Err(AppError::Other("不支持或不完整的删除快照".into()));
    }
    let mut seen = HashSet::new();
    for member in &manifest.members {
        let relative = paths::checked_relative_path(&member.path)?;
        let canonical_name = relative.to_string_lossy().replace('\\', "/");
        if canonical_name != member.path
            || !(DATABASES.contains(&member.path.as_str())
                || METADATA.contains(&member.path.as_str())
                || ((member.path.starts_with("sessions/")
                    || member.path.starts_with("archived_sessions/"))
                    && member.path.ends_with(".jsonl")))
        {
            return Err(AppError::Path("快照成员超出删除数据范围".into()));
        }
        let normalized = relative.to_string_lossy().replace('\\', "/").to_lowercase();
        if !seen.insert(normalized) {
            return Err(AppError::Path("快照包含重复路径".into()));
        }
        if member.sqlite != DATABASES.contains(&member.path.as_str()) {
            return Err(AppError::Other("快照数据库类型不一致".into()));
        }
    }
    for required in DATABASES.iter().chain(METADATA) {
        if !manifest
            .members
            .iter()
            .any(|member| member.path == *required)
        {
            return Err(AppError::Other(format!("删除快照缺少成员: {required}")));
        }
    }
    if !manifest
        .members
        .iter()
        .any(|member| member.path == "state_5.sqlite" && member.content.is_some())
    {
        return Err(AppError::Other("删除快照缺少 Core 数据库".into()));
    }
    Ok(manifest)
}

fn load_verified(snapshot: &Path) -> AppResult<Manifest> {
    let manifest = load_manifest(snapshot)?;
    for member in &manifest.members {
        let relative = paths::checked_relative_path(&member.path)?;
        let path = snapshot.join("files").join(relative);
        let present =
            path_safety::validate_descendant(snapshot, &path, EntryKind::File, true, "快照内容")?;
        match &member.content {
            Some(expected) if present && digest(&path)? == *expected => {
                if member.sqlite {
                    check_database(&path)?;
                }
            }
            None if !present => {}
            _ => {
                return Err(AppError::Other(format!(
                    "删除快照完整性校验失败: {}",
                    member.path
                )))
            }
        }
    }
    Ok(manifest)
}

pub fn verify(snapshot_path: &Path) -> AppResult<SnapshotReport> {
    let manifest = load_verified(snapshot_path)?;
    Ok(SnapshotReport {
        snapshot_path: snapshot_path.to_string_lossy().into_owned(),
        verified: true,
        files: manifest
            .members
            .iter()
            .filter(|member| member.content.is_some())
            .count(),
    })
}

pub fn restore_to_new_dir(snapshot_path: &Path, output: &Path) -> AppResult<SnapshotReport> {
    // Validate the entire package before creating anything. No in-place or merge recovery exists.
    let manifest = load_verified(snapshot_path)?;
    let snapshot = snapshot_path.canonicalize()?;
    if output.try_exists()? {
        return Err(AppError::Path("恢复目标必须是尚不存在的新目录".into()));
    }
    let output = resolved_destination(output)?;
    let source = PathBuf::from(&manifest.source_root);
    if within(&output, &snapshot) || within(&output, &source) {
        return Err(AppError::Path(
            "隔离恢复目标不能位于快照或原生数据目录内".into(),
        ));
    }
    // Require an existing parent: never create a user-selected ancestor tree as a side effect.
    let parent = output
        .parent()
        .ok_or_else(|| AppError::Path("恢复目标缺少父目录".into()))?;
    if !parent.is_dir() {
        return Err(AppError::Path("恢复目标的父目录必须已存在".into()));
    }
    fs::create_dir(&output)?; // create-if-absent is also the final race check.
    sync_directory(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o700))?;
    }
    let operation = (|| -> AppResult<()> {
        for member in &manifest.members {
            if let Some(expected) = &member.content {
                let relative = paths::checked_relative_path(&member.path)?;
                let source = snapshot.join("files").join(&relative);
                path_safety::validate_descendant(
                    &snapshot,
                    &source,
                    EntryKind::File,
                    false,
                    "恢复快照内容",
                )?;
                let destination = output.join(relative);
                copy_new(&source, &destination)?;
                if digest(&destination)? != *expected {
                    return Err(AppError::Other("恢复期间快照发生变化".into()));
                }
                if member.sqlite {
                    check_database(&destination)?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = operation {
        return Err(AppError::Other(format!(
            "隔离恢复未完成（不得用于原生覆盖）: {error}；部分文件保留在 {}",
            output.display()
        )));
    }
    // This marker is published only after every restored file was reopened and verified.
    atomic_file::create_with_writer_if_absent(
        &output.join("agentvault-delete-recovery.json"),
        |file| {
            serde_json::to_writer_pretty(file, &manifest)?;
            Ok(())
        },
    )?;
    Ok(SnapshotReport {
        snapshot_path: snapshot.to_string_lossy().into_owned(),
        verified: true,
        files: manifest
            .members
            .iter()
            .filter(|member| member.content.is_some())
            .count(),
    })
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum TestFault {
    Corrupt,
    SourceChanged,
    LateRollout,
}
#[cfg(test)]
thread_local! { static TEST_FAULT: std::cell::Cell<Option<TestFault>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(crate) fn fail_next(fault: TestFault) {
    TEST_FAULT.with(|value| value.set(Some(fault)));
}
#[cfg(test)]
fn inject_fault(snapshot: &Path, source: &Path) -> AppResult<()> {
    match TEST_FAULT.with(|value| value.take()) {
        Some(TestFault::Corrupt) => fs::write(snapshot.join("files/state_5.sqlite"), b"corrupt")?,
        Some(TestFault::SourceChanged) => {
            fs::write(source.join("session_index.jsonl"), b"concurrent writer\n")?
        }
        Some(TestFault::LateRollout) => fail_next(TestFault::LateRollout),
        None => {}
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn inject_before_removal(path: &Path) -> AppResult<()> {
    if matches!(
        TEST_FAULT.with(|value| value.take()),
        Some(TestFault::LateRollout)
    ) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(b"concurrent append after snapshot\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod management_tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> AppResult<Self> {
            let root = std::env::temp_dir().join(unique_name()?);
            fs::create_dir(&root)?;
            Ok(Self(root.canonicalize()?))
        }
        fn package(&self, name: &str) -> AppResult<PathBuf> {
            let snapshot = self.0.join("backups/codex-delete-snapshots").join(name);
            fs::create_dir_all(snapshot.join("files"))?;
            let database = snapshot.join("files/state_5.sqlite");
            Connection::open(&database)?.execute_batch(
                "CREATE TABLE threads (id TEXT); INSERT INTO threads VALUES ('fixture');",
            )?;
            let members = DATABASES
                .iter()
                .chain(METADATA)
                .map(|path| Member {
                    path: (*path).into(),
                    sqlite: DATABASES.contains(path),
                    content: if *path == "state_5.sqlite" {
                        Some(digest(&database).unwrap())
                    } else {
                        None
                    },
                })
                .collect();
            let manifest = Manifest {
                version: 1,
                source_root: self.0.join("source").to_string_lossy().into_owned(),
                ids: vec!["fixture".into()],
                members,
            };
            fs::write(
                snapshot.join("manifest.json"),
                serde_json::to_vec(&manifest)?,
            )?;
            Ok(snapshot)
        }
        fn backup(&self) -> String {
            self.0.join("backups").to_string_lossy().into_owned()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn expected_file_resolves_ancestor_aliases_for_present_and_absent_metadata() -> AppResult<()> {
        let fixture = Fixture::new()?;
        let source = fixture.0.join("source");
        fs::create_dir_all(source.join("child"))?;
        Connection::open(source.join("state_5.sqlite"))?
            .execute_batch("CREATE TABLE threads (id TEXT);")?;
        fs::write(source.join("session_index.jsonl"), b"fixture\n")?;
        let snapshot = Preparation::new(&source, &fixture.0.join("backups"))?
            .finish(&["fixture".into()], &[])?;
        let alias = source.join("child").join("..");
        assert_eq!(
            snapshot.expected_file(&alias.join("session_index.jsonl"))?,
            Some(atomic_file::fingerprint(
                &source.join("session_index.jsonl")
            )?)
        );
        assert_eq!(snapshot.expected_file(&alias.join("history.jsonl"))?, None);
        assert!(snapshot
            .expected_file(&alias.join("unobserved.jsonl"))
            .is_err());
        Ok(())
    }

    #[test]
    fn management_list_isolates_broken_and_missing_packages_without_claiming_verification(
    ) -> AppResult<()> {
        let fixture = Fixture::new()?;
        let good = fixture.package("delete-20260909T010203Z-good")?;
        let bad = fixture.package("delete-bad")?;
        fs::write(bad.join("manifest.json"), b"not json")?;
        let missing_manifest = fixture.package("delete-missing-manifest")?;
        fs::remove_file(missing_manifest.join("manifest.json"))?;
        let missing_payload = fixture.package("delete-missing-payload")?;
        fs::remove_file(missing_payload.join("files/state_5.sqlite"))?;
        let list = list_delete_snapshots(&fixture.backup())?;
        assert_eq!(list.len(), 4);
        let status = |name: &str| list.iter().find(|item| item.name == name).unwrap().status;
        assert_eq!(status("delete-bad"), "unreadable");
        assert_eq!(status("delete-missing-manifest"), "incomplete");
        assert_eq!(status("delete-missing-payload"), "incomplete");
        assert_eq!(status("delete-20260909T010203Z-good"), "unverified");
        let detail = inspect_delete_snapshot(&fixture.backup(), &good.to_string_lossy())?;
        assert_eq!(
            detail.summary.created_at.as_deref(),
            Some("2026-09-09T01:02:03+00:00")
        );
        assert_eq!(detail.summary.files, 1);
        assert!(detail.summary.total_bytes > 0);
        assert!(detail
            .members
            .iter()
            .any(|item| item.path == "state_5.sqlite" && item.present && item.sqlite));
        // Deliberate payload corruption remains unverified until an explicit full check.
        fs::write(good.join("files/state_5.sqlite"), b"tampered")?;
        assert_eq!(
            inspect_delete_snapshot(&fixture.backup(), &good.to_string_lossy())?
                .summary
                .status,
            "unverified"
        );
        assert!(verify_delete_snapshot(&fixture.backup(), &good.to_string_lossy()).is_err());
        Ok(())
    }

    #[test]
    fn management_rejects_root_outside_nested_and_manifest_path_injection() -> AppResult<()> {
        let fixture = Fixture::new()?;
        let good = fixture.package("delete-good")?;
        let root = good.parent().unwrap();
        for invalid in [
            root.to_path_buf(),
            fixture.0.clone(),
            good.join("files"),
            // Windows verbatim PathBuf::join normalizes '..'; preserve raw input.
            PathBuf::from(format!(
                "{}/../codex-delete-snapshots/delete-good",
                paths::strip_verbatim(&root.to_string_lossy())
            )),
        ] {
            assert!(
                inspect_delete_snapshot(&fixture.backup(), &invalid.to_string_lossy()).is_err(),
                "accepted {}",
                invalid.display()
            );
            assert!(verify_delete_snapshot(&fixture.backup(), &invalid.to_string_lossy()).is_err());
            assert!(restore_delete_snapshot(
                &fixture.backup(),
                &invalid.to_string_lossy(),
                &fixture.0.join("output").to_string_lossy()
            )
            .is_err());
        }
        assert!(!fixture.0.join("output").exists());
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(good.join("manifest.json"))?)?;
        manifest.members.push(Member {
            path: "../escape.jsonl".into(),
            sqlite: false,
            content: None,
        });
        fs::write(good.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
        assert_eq!(
            inspect_delete_snapshot(&fixture.backup(), &good.to_string_lossy())?
                .summary
                .status,
            "unreadable"
        );
        assert!(list_delete_snapshots(&format!(
            "{}/backups/../backups",
            paths::strip_verbatim(&fixture.0.to_string_lossy())
        ))
        .is_err());
        Ok(())
    }

    #[test]
    fn management_restore_verifies_output_and_refuses_existing_target() -> AppResult<()> {
        let fixture = Fixture::new()?;
        let good = fixture.package("delete-good")?;
        assert!(verify_delete_snapshot(&fixture.backup(), &good.to_string_lossy())?.verified);
        let output = fixture.0.join("restored");
        let report = restore_delete_snapshot(
            &fixture.backup(),
            &good.to_string_lossy(),
            &output.to_string_lossy(),
        )?;
        assert!(report.verified);
        assert_eq!(
            fs::read(good.join("files/state_5.sqlite"))?,
            fs::read(output.join("state_5.sqlite"))?
        );
        assert!(output.join("agentvault-delete-recovery.json").is_file());
        assert!(restore_delete_snapshot(
            &fixture.backup(),
            &good.to_string_lossy(),
            &output.to_string_lossy()
        )
        .is_err());
        assert_eq!(
            list_delete_snapshots(&fixture.backup())?[0].status,
            "unverified"
        );
        Ok(())
    }
}
