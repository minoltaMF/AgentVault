use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use provider_sdk::ResumePlan;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use vault_io::atomic::create_with_writer_if_absent;
use vault_io::path_safety::{
    checked_relative_path, metadata_is_link_or_reparse, validate_descendant, EntryKind,
};

pub const SNAPSHOT_MANIFEST_SCHEMA_VERSION: u32 = 1;

pub type ManifestResult<T> = Result<T, ManifestError>;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("snapshot manifest JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot manifest I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot manifest atomic I/O failed: {0}")]
    VaultIo(#[from] vault_io::Error),
    #[error("unsupported snapshot manifest schema version {found}")]
    UnsupportedSchemaVersion { found: u32 },
    #[error("snapshot manifest timestamp is invalid: {0}")]
    InvalidTimestamp(#[from] chrono::ParseError),
    #[error("snapshot manifest field {field} must be non-empty and contain no control characters")]
    InvalidField { field: &'static str },
    #[error("snapshot manifest SHA-256 must contain exactly 64 hexadecimal characters")]
    InvalidSha256,
    #[error("snapshot object id must use the sha256:<digest> form")]
    InvalidObjectId,
    #[error("snapshot member path is unsafe ({path}): {reason}")]
    InvalidRelativePath { path: String, reason: String },
    #[error("chunked snapshot storage must contain at least one object id")]
    EmptyChunkList,
    #[error("whole-object storage id must match the member SHA-256")]
    ObjectDigestMismatch,
    #[error("snapshot manifest must contain at least one member")]
    EmptyMembers,
    #[error("snapshot manifest contains the same relative path more than once: {0}")]
    DuplicateMemberPath(String),
    #[error("snapshot cannot reference itself as its previous snapshot")]
    PreviousSnapshotSelfReference,
    #[error("snapshot id in {field} is not a single safe path component")]
    InvalidSnapshotId { field: &'static str },
    #[error("snapshot manifest root must be an existing ordinary directory: {0}")]
    InvalidSnapshotRoot(PathBuf),
    #[error("snapshot manifest path id {expected} does not match document id {actual}")]
    SnapshotPathIdMismatch { expected: String, actual: String },
    #[error("captured resume plan field {field} is not valid UTF-8")]
    NonUtf8ResumeField { field: &'static str },
}

/// A binary SHA-256 digest with one canonical lowercase hexadecimal representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl FromStr for Sha256Digest {
    type Err = ManifestError;

    fn from_str(value: &str) -> ManifestResult<Self> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ManifestError::InvalidSha256);
        }

        let mut digest = [0_u8; 32];
        for (index, output) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            let high = hex_nibble(value.as_bytes()[offset]).ok_or(ManifestError::InvalidSha256)?;
            let low =
                hex_nibble(value.as_bytes()[offset + 1]).ok_or(ManifestError::InvalidSha256)?;
            *output = (high << 4) | low;
        }
        Ok(Self(digest))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

/// A content-addressed object reference. Object persistence is implemented separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    digest: Sha256Digest,
}

impl ObjectId {
    pub const fn sha256(digest: Sha256Digest) -> Self {
        Self { digest }
    }

    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

impl FromStr for ObjectId {
    type Err = ManifestError;

    fn from_str(value: &str) -> ManifestResult<Self> {
        let digest = value
            .strip_prefix("sha256:")
            .ok_or(ManifestError::InvalidObjectId)?;
        Sha256Digest::from_str(digest)
            .map(Self::sha256)
            .map_err(|_| ManifestError::InvalidObjectId)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sha256:{}", self.digest)
    }
}

impl Serialize for ObjectId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SnapshotStorageRepr {
    Object { object: ObjectId },
    Chunked { chunks: Vec<ObjectId> },
}

/// How one logical member is reconstructed from immutable object references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SnapshotStorage(SnapshotStorageRepr);

impl<'de> Deserialize<'de> for SnapshotStorage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let storage = Self(SnapshotStorageRepr::deserialize(deserializer)?);
        storage.validate().map_err(de::Error::custom)?;
        Ok(storage)
    }
}

impl SnapshotStorage {
    pub const fn object(object: ObjectId) -> Self {
        Self(SnapshotStorageRepr::Object { object })
    }

    pub fn chunked(chunks: Vec<ObjectId>) -> ManifestResult<Self> {
        let storage = Self(SnapshotStorageRepr::Chunked { chunks });
        storage.validate()?;
        Ok(storage)
    }

    pub const fn object_id(&self) -> Option<&ObjectId> {
        match &self.0 {
            SnapshotStorageRepr::Object { object } => Some(object),
            SnapshotStorageRepr::Chunked { .. } => None,
        }
    }

    pub fn chunks(&self) -> Option<&[ObjectId]> {
        match &self.0 {
            SnapshotStorageRepr::Object { .. } => None,
            SnapshotStorageRepr::Chunked { chunks } => Some(chunks),
        }
    }

    fn validate(&self) -> ManifestResult<()> {
        if matches!(&self.0, SnapshotStorageRepr::Chunked { chunks } if chunks.is_empty()) {
            Err(ManifestError::EmptyChunkList)
        } else {
            Ok(())
        }
    }
}

/// Consistency evidence established while native data was captured.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotConsistency {
    Verified,
    VerifiedPrefix,
    Fuzzy,
}

/// The safe, serializable subset of an inert provider-native resume plan.
///
/// Environment values, terminal selection, and execution state are deliberately not captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapturedResumePlan {
    executable: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

impl CapturedResumePlan {
    pub fn try_new(
        executable: impl Into<String>,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> ManifestResult<Self> {
        let plan = Self {
            executable: executable.into(),
            args,
            cwd,
        };
        require_text("resume_plan_at_capture.executable", &plan.executable)?;
        for argument in &plan.args {
            require_no_nul("resume_plan_at_capture.args", argument)?;
        }
        if let Some(cwd) = &plan.cwd {
            require_text("resume_plan_at_capture.cwd", cwd)?;
        }
        Ok(plan)
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
}

impl TryFrom<&ResumePlan> for CapturedResumePlan {
    type Error = ManifestError;

    fn try_from(plan: &ResumePlan) -> ManifestResult<Self> {
        let executable = utf8_path(&plan.executable, "resume_plan_at_capture.executable")?;
        let args = plan
            .args
            .iter()
            .map(|argument| {
                argument
                    .to_str()
                    .map(str::to_owned)
                    .ok_or(ManifestError::NonUtf8ResumeField {
                        field: "resume_plan_at_capture.args",
                    })
            })
            .collect::<ManifestResult<Vec<_>>>()?;
        let cwd = plan
            .selected_cwd
            .as_deref()
            .map(|path| utf8_path(path, "resume_plan_at_capture.cwd"))
            .transpose()?;
        Self::try_new(executable, args, cwd)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedResumePlanDocument {
    executable: String,
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

impl TryFrom<CapturedResumePlanDocument> for CapturedResumePlan {
    type Error = ManifestError;

    fn try_from(document: CapturedResumePlanDocument) -> ManifestResult<Self> {
        Self::try_new(document.executable, document.args, document.cwd)
    }
}

/// One logical file included in a snapshot manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotMember {
    role: String,
    relative_path: String,
    size: u64,
    sha256: Sha256Digest,
    storage: SnapshotStorage,
}

impl SnapshotMember {
    pub fn try_new(
        role: impl Into<String>,
        relative_path: impl Into<String>,
        size: u64,
        sha256: Sha256Digest,
        storage: SnapshotStorage,
    ) -> ManifestResult<Self> {
        let role = role.into();
        require_text("members.role", &role)?;
        let relative_path = normalize_relative_path(relative_path.into())?;
        storage.validate()?;
        if storage
            .object_id()
            .is_some_and(|object| object.digest() != &sha256)
        {
            return Err(ManifestError::ObjectDigestMismatch);
        }
        Ok(Self {
            role,
            relative_path,
            size,
            sha256,
            storage,
        })
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub const fn size(&self) -> u64 {
        self.size
    }

    pub const fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub const fn storage(&self) -> &SnapshotStorage {
        &self.storage
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotMemberDocument {
    role: String,
    relative_path: String,
    size: u64,
    sha256: Sha256Digest,
    storage: SnapshotStorage,
}

impl TryFrom<SnapshotMemberDocument> for SnapshotMember {
    type Error = ManifestError;

    fn try_from(document: SnapshotMemberDocument) -> ManifestResult<Self> {
        Self::try_new(
            document.role,
            document.relative_path,
            document.size,
            document.sha256,
            document.storage,
        )
    }
}

/// Mutable construction input consumed to produce an immutable, validated manifest.
#[derive(Debug, Clone)]
pub struct SnapshotManifestDraft {
    pub snapshot_id: String,
    pub machine_id: String,
    pub created_at: DateTime<Utc>,
    pub reason: String,
    pub provider: String,
    pub source_instance_id: String,
    pub native_session_id: String,
    pub project_id: Option<String>,
    pub consistency: SnapshotConsistency,
    pub parser_version: String,
    pub previous_snapshot_id: Option<String>,
    pub resume_plan_at_capture: Option<CapturedResumePlan>,
    pub git_checkpoint_id: Option<String>,
    pub members: Vec<SnapshotMember>,
}

/// Snapshot manifest v1 after structural validation and canonical member ordering.
///
/// Fields are private and the type exposes no mutators. Deserialization is routed through
/// [`SnapshotManifest::from_json_slice`] so untrusted documents cannot bypass invariants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotManifest {
    schema_version: u32,
    snapshot_id: String,
    machine_id: String,
    #[serde(serialize_with = "serialize_timestamp")]
    created_at: DateTime<Utc>,
    reason: String,
    provider: String,
    source_instance_id: String,
    native_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    consistency: SnapshotConsistency,
    parser_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_plan_at_capture: Option<CapturedResumePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_checkpoint_id: Option<String>,
    members: Vec<SnapshotMember>,
}

impl SnapshotManifest {
    pub fn try_new(mut draft: SnapshotManifestDraft) -> ManifestResult<Self> {
        require_text("snapshot_id", &draft.snapshot_id)?;
        require_snapshot_id("snapshot_id", &draft.snapshot_id)?;
        require_text("machine_id", &draft.machine_id)?;
        require_text("reason", &draft.reason)?;
        require_text("provider", &draft.provider)?;
        require_text("source_instance_id", &draft.source_instance_id)?;
        require_text("native_session_id", &draft.native_session_id)?;
        require_text("parser_version", &draft.parser_version)?;
        require_optional_text("project_id", draft.project_id.as_deref())?;
        require_optional_text(
            "previous_snapshot_id",
            draft.previous_snapshot_id.as_deref(),
        )?;
        if let Some(previous) = draft.previous_snapshot_id.as_deref() {
            require_snapshot_id("previous_snapshot_id", previous)?;
        }
        require_optional_text("git_checkpoint_id", draft.git_checkpoint_id.as_deref())?;

        if draft.previous_snapshot_id.as_deref() == Some(draft.snapshot_id.as_str()) {
            return Err(ManifestError::PreviousSnapshotSelfReference);
        }
        if draft.members.is_empty() {
            return Err(ManifestError::EmptyMembers);
        }

        draft.members.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| left.role.cmp(&right.role))
        });
        if let Some(duplicate) = draft.members.windows(2).find_map(|members| {
            (members[0].relative_path == members[1].relative_path)
                .then(|| members[0].relative_path.clone())
        }) {
            return Err(ManifestError::DuplicateMemberPath(duplicate));
        }

        Ok(Self {
            schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
            snapshot_id: draft.snapshot_id,
            machine_id: draft.machine_id,
            created_at: draft.created_at,
            reason: draft.reason,
            provider: draft.provider,
            source_instance_id: draft.source_instance_id,
            native_session_id: draft.native_session_id,
            project_id: draft.project_id,
            consistency: draft.consistency,
            parser_version: draft.parser_version,
            previous_snapshot_id: draft.previous_snapshot_id,
            resume_plan_at_capture: draft.resume_plan_at_capture,
            git_checkpoint_id: draft.git_checkpoint_id,
            members: draft.members,
        })
    }

    pub fn from_json_slice(bytes: &[u8]) -> ManifestResult<Self> {
        serde_json::from_slice::<SnapshotManifestDocument>(bytes)?.try_into()
    }

    /// Serialize a stable, human-readable v1 document with a trailing newline.
    pub fn to_json_bytes(&self) -> ManifestResult<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub const fn created_at(&self) -> &DateTime<Utc> {
        &self.created_at
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn source_instance_id(&self) -> &str {
        &self.source_instance_id
    }

    pub fn native_session_id(&self) -> &str {
        &self.native_session_id
    }

    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    pub const fn consistency(&self) -> SnapshotConsistency {
        self.consistency
    }

    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    pub fn previous_snapshot_id(&self) -> Option<&str> {
        self.previous_snapshot_id.as_deref()
    }

    pub const fn resume_plan_at_capture(&self) -> Option<&CapturedResumePlan> {
        self.resume_plan_at_capture.as_ref()
    }

    pub fn git_checkpoint_id(&self) -> Option<&str> {
        self.git_checkpoint_id.as_deref()
    }

    pub fn members(&self) -> &[SnapshotMember] {
        &self.members
    }
}

/// A write-once manifest repository rooted at an explicitly selected `snapshots` directory.
///
/// The store never chooses or creates an application data root. Each manifest is atomically
/// created at `<snapshots_root>/<snapshot_id>/manifest.json`; an existing manifest is a conflict
/// and is never replaced.
#[derive(Debug, Clone)]
pub struct SnapshotManifestStore {
    snapshots_root: PathBuf,
}

impl SnapshotManifestStore {
    pub fn open(snapshots_root: impl Into<PathBuf>) -> ManifestResult<Self> {
        let snapshots_root = snapshots_root.into();
        let metadata = fs::symlink_metadata(&snapshots_root)?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(ManifestError::InvalidSnapshotRoot(snapshots_root));
        }
        Ok(Self { snapshots_root })
    }

    pub fn snapshots_root(&self) -> &Path {
        &self.snapshots_root
    }

    pub fn write_new(&self, manifest: &SnapshotManifest) -> ManifestResult<PathBuf> {
        let snapshot_dir = self.snapshot_dir(manifest.snapshot_id())?;
        match fs::create_dir(&snapshot_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        validate_descendant(
            &self.snapshots_root,
            &snapshot_dir,
            EntryKind::Directory,
            false,
            "snapshot manifest directory",
        )?;

        let manifest_path = snapshot_dir.join("manifest.json");
        validate_descendant(
            &self.snapshots_root,
            &manifest_path,
            EntryKind::File,
            true,
            "snapshot manifest",
        )?;
        let bytes = manifest.to_json_bytes()?;
        create_with_writer_if_absent(&manifest_path, |file| {
            file.write_all(&bytes)?;
            Ok(())
        })?;
        Ok(manifest_path)
    }

    pub fn read(&self, snapshot_id: &str) -> ManifestResult<SnapshotManifest> {
        let manifest_path = self.snapshot_dir(snapshot_id)?.join("manifest.json");
        validate_descendant(
            &self.snapshots_root,
            &manifest_path,
            EntryKind::File,
            false,
            "snapshot manifest",
        )?;
        let manifest = SnapshotManifest::from_json_slice(&fs::read(manifest_path)?)?;
        if manifest.snapshot_id() != snapshot_id {
            return Err(ManifestError::SnapshotPathIdMismatch {
                expected: snapshot_id.to_string(),
                actual: manifest.snapshot_id().to_string(),
            });
        }
        Ok(manifest)
    }

    fn snapshot_dir(&self, snapshot_id: &str) -> ManifestResult<PathBuf> {
        require_snapshot_id("snapshot_id", snapshot_id)?;
        Ok(self.snapshots_root.join(snapshot_id))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotManifestDocument {
    schema_version: u32,
    snapshot_id: String,
    machine_id: String,
    created_at: String,
    reason: String,
    provider: String,
    source_instance_id: String,
    native_session_id: String,
    #[serde(default)]
    project_id: Option<String>,
    consistency: SnapshotConsistency,
    parser_version: String,
    #[serde(default)]
    previous_snapshot_id: Option<String>,
    #[serde(default)]
    resume_plan_at_capture: Option<CapturedResumePlanDocument>,
    #[serde(default)]
    git_checkpoint_id: Option<String>,
    members: Vec<SnapshotMemberDocument>,
}

impl TryFrom<SnapshotManifestDocument> for SnapshotManifest {
    type Error = ManifestError;

    fn try_from(document: SnapshotManifestDocument) -> ManifestResult<Self> {
        if document.schema_version != SNAPSHOT_MANIFEST_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion {
                found: document.schema_version,
            });
        }
        let created_at = DateTime::parse_from_rfc3339(&document.created_at)?.with_timezone(&Utc);
        let resume_plan_at_capture = document
            .resume_plan_at_capture
            .map(CapturedResumePlan::try_from)
            .transpose()?;
        let members = document
            .members
            .into_iter()
            .map(SnapshotMember::try_from)
            .collect::<ManifestResult<Vec<_>>>()?;
        Self::try_new(SnapshotManifestDraft {
            snapshot_id: document.snapshot_id,
            machine_id: document.machine_id,
            created_at,
            reason: document.reason,
            provider: document.provider,
            source_instance_id: document.source_instance_id,
            native_session_id: document.native_session_id,
            project_id: document.project_id,
            consistency: document.consistency,
            parser_version: document.parser_version,
            previous_snapshot_id: document.previous_snapshot_id,
            resume_plan_at_capture,
            git_checkpoint_id: document.git_checkpoint_id,
            members,
        })
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_relative_path(raw: String) -> ManifestResult<String> {
    let path = checked_relative_path(&raw).map_err(|error| ManifestError::InvalidRelativePath {
        path: raw,
        reason: error.to_string(),
    })?;
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn require_text(field: &'static str, value: &str) -> ManifestResult<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(ManifestError::InvalidField { field })
    } else {
        Ok(())
    }
}

fn require_no_nul(field: &'static str, value: &str) -> ManifestResult<()> {
    if value.contains('\0') {
        Err(ManifestError::InvalidField { field })
    } else {
        Ok(())
    }
}

fn require_optional_text(field: &'static str, value: Option<&str>) -> ManifestResult<()> {
    value.map_or(Ok(()), |value| require_text(field, value))
}

fn require_snapshot_id(field: &'static str, value: &str) -> ManifestResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.trim() != value
        || value.contains(['/', '\\', ':'])
        || value.chars().any(char::is_control)
    {
        Err(ManifestError::InvalidSnapshotId { field })
    } else {
        Ok(())
    }
}

fn utf8_path(path: &Path, field: &'static str) -> ManifestResult<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(ManifestError::NonUtf8ResumeField { field })
}

fn serialize_timestamp<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}
