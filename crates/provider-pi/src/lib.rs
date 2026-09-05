//! Read-only Pi session discovery and v1/v2/v3 branch graph parsing.
//!
//! Pi stores append-only JSONL sessions below `~/.pi/agent/sessions`. This crate treats the
//! configured provider root as `~/.pi/agent`, never writes native files, and retains each parsed
//! record's raw JSON value so extension events are not discarded.

use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use provider_sdk::{
    BranchGraph, BranchGraphError, BranchNode, ConsistencyClass, DetectionContext, DetectionResult,
    DiscoveryError, DiscoveryPage, DiscoveryResult, NativeSessionKind, NativeSessionRef,
    ProviderCapabilities, ProviderContext, ProviderDescriptor, ScanCursor, SessionProvider,
    SessionRoot, SessionRootKind, SourceKind,
};
use serde_json::{Map, Value};

const PROVIDER_ID: &str = "pi";

#[derive(Debug, Default, Clone, Copy)]
pub struct PiProvider;

impl PiProvider {
    /// Read and parse one previously discovered Pi session without modifying it.
    pub fn read_session(
        &self,
        context: &ProviderContext<'_>,
        native: &NativeSessionRef,
    ) -> PiSessionResult<PiSession> {
        ensure_not_cancelled(context)?;
        validate_native_ref(context, native)?;
        let contents = fs::read_to_string(&native.source_path)
            .map_err(|source| PiSessionError::io(&native.source_path, source))?;
        ensure_not_cancelled(context)?;
        let session = parse_session(&native.source_path, &contents, context)?;

        if let Some(expected_id) = native.native_session_id.as_deref() {
            if expected_id != session.header.id.as_str() {
                return Err(PiSessionError::IdentityMismatch {
                    expected: expected_id.to_string(),
                    actual: session.header.id.clone(),
                });
            }
        }
        if let Some(expected_version) = native.source_format_version.as_deref() {
            let actual_version = session.header.version.to_string();
            if expected_version != actual_version.as_str() {
                return Err(PiSessionError::FormatVersionMismatch {
                    expected: expected_version.to_string(),
                    actual: session.header.version,
                });
            }
        }
        Ok(session)
    }
}

impl SessionProvider for PiProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            display_name: "Pi".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            native_cli: Some("pi".into()),
            capabilities: ProviderCapabilities::DISCOVER
                | ProviderCapabilities::PARSE
                | ProviderCapabilities::BRANCH_GRAPH,
            consistency_classes: vec![
                ConsistencyClass::AppendOnlyJsonl,
                ConsistencyClass::DirectoryTree,
            ],
            source_kinds: vec![SourceKind::Local, SourceKind::Wsl],
            health_probe_timeout_ms: 1_000,
        }
    }

    fn detect(&self, context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult> {
        let sessions = sessions_root(context.root());
        let evidence = match ordinary_directory(&sessions)? {
            true => vec![sessions],
            false => Vec::new(),
        };
        Ok(DetectionResult::from_evidence(evidence))
    }

    fn roots(&self, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<SessionRoot>> {
        context.ensure_not_cancelled()?;
        Ok(vec![session_root(context.root())])
    }

    fn discover(
        &self,
        context: &ProviderContext<'_>,
        cursor: Option<ScanCursor>,
    ) -> DiscoveryResult<DiscoveryPage> {
        reject_cursor(cursor)?;
        context.ensure_not_cancelled()?;
        let root = session_root(context.root());
        let mut sessions = Vec::new();
        for source_path in session_paths(&root.path, context)? {
            context.ensure_not_cancelled()?;
            let header = probe_header(&source_path, context)?;
            let native_id = header
                .as_ref()
                .and_then(|header| header.id.clone())
                .or_else(|| session_id_from_filename(&source_path));
            let mut native = NativeSessionRef::new(
                PROVIDER_ID,
                native_id,
                source_path,
                root.clone(),
                NativeSessionKind::Primary,
            );
            if let Some(version) = header.and_then(|header| header.version) {
                native = native.with_source_format_version(version.to_string());
            }
            sessions.push(native);
        }
        sessions.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(DiscoveryPage::complete(sessions))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiSessionHeader {
    pub version: u32,
    pub id: String,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
    pub parent_session: Option<String>,
    pub raw: Value,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiSessionEntry {
    pub node: BranchNode,
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub raw: Value,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiPartialTail {
    pub line_number: usize,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiSession {
    pub header: PiSessionHeader,
    pub entries: Vec<PiSessionEntry>,
    pub graph: BranchGraph,
    /// An unterminated final line can be observed while Pi is appending to an active session.
    /// It is retained verbatim but excluded from the graph until it becomes valid JSON.
    pub partial_tail: Option<PiPartialTail>,
}

#[derive(Debug)]
pub enum PiSessionError {
    Cancelled,
    Io {
        path: PathBuf,
        source: io::Error,
    },
    UnsafePath {
        path: PathBuf,
        reason: String,
    },
    ProviderMismatch {
        actual: String,
    },
    IdentityMismatch {
        expected: String,
        actual: String,
    },
    FormatVersionMismatch {
        expected: String,
        actual: u32,
    },
    InvalidHeader {
        path: PathBuf,
        line_number: usize,
        reason: String,
    },
    UnsupportedVersion {
        version: u32,
    },
    InvalidRecord {
        path: PathBuf,
        line_number: usize,
        reason: String,
    },
    BranchGraph(BranchGraphError),
}

impl PiSessionError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    fn invalid_header(path: &Path, line_number: usize, reason: impl Into<String>) -> Self {
        Self::InvalidHeader {
            path: path.to_path_buf(),
            line_number,
            reason: reason.into(),
        }
    }

    fn invalid_record(path: &Path, line_number: usize, reason: impl Into<String>) -> Self {
        Self::InvalidRecord {
            path: path.to_path_buf(),
            line_number,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for PiSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Pi session read cancelled"),
            Self::Io { path, source } => {
                write!(formatter, "failed to read Pi session {}: {source}", path.display())
            }
            Self::UnsafePath { path, reason } => write!(
                formatter,
                "rejected unsafe Pi session path {}: {reason}",
                path.display()
            ),
            Self::ProviderMismatch { actual } => {
                write!(formatter, "expected Pi provider reference, got {actual}")
            }
            Self::IdentityMismatch { expected, actual } => write!(
                formatter,
                "Pi session id changed between discovery and parsing: expected {expected}, got {actual}"
            ),
            Self::FormatVersionMismatch { expected, actual } => write!(
                formatter,
                "Pi session format changed between discovery and parsing: expected {expected}, got {actual}"
            ),
            Self::InvalidHeader {
                path,
                line_number,
                reason,
            } => write!(
                formatter,
                "invalid Pi session header at {}:{line_number}: {reason}",
                path.display()
            ),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported Pi session format version: {version}")
            }
            Self::InvalidRecord {
                path,
                line_number,
                reason,
            } => write!(
                formatter,
                "invalid Pi session record at {}:{line_number}: {reason}",
                path.display()
            ),
            Self::BranchGraph(source) => write!(formatter, "invalid Pi branch graph: {source}"),
        }
    }
}

impl Error for PiSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::BranchGraph(source) => Some(source),
            _ => None,
        }
    }
}

impl From<BranchGraphError> for PiSessionError {
    fn from(source: BranchGraphError) -> Self {
        Self::BranchGraph(source)
    }
}

pub type PiSessionResult<T> = Result<T, PiSessionError>;

#[derive(Debug)]
struct HeaderProbe {
    id: Option<String>,
    version: Option<u32>,
}

fn sessions_root(pi_agent_home: &Path) -> PathBuf {
    pi_agent_home.join("sessions")
}

fn session_root(pi_agent_home: &Path) -> SessionRoot {
    SessionRoot::new(
        sessions_root(pi_agent_home),
        SessionRootKind::Active,
        ConsistencyClass::AppendOnlyJsonl,
    )
}

fn ordinary_directory(path: &Path) -> DiscoveryResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(DiscoveryError::io(path, error)),
    };
    if vault_io::path_safety::metadata_is_link_or_reparse(&metadata) {
        return Err(DiscoveryError::unsafe_path(
            path,
            "session root is a symlink or reparse point",
        ));
    }
    Ok(metadata.is_dir())
}

fn session_paths(root: &Path, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<PathBuf>> {
    if !ordinary_directory(root)? {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        context.ensure_not_cancelled()?;
        let entry = entry.map_err(|error| {
            DiscoveryError::traversal(root, format!("failed to walk Pi sessions: {error}"))
        })?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| DiscoveryError::io(entry.path(), error))?;
        if vault_io::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(DiscoveryError::unsafe_path(
                entry.path(),
                "sessions tree contains a symlink or reparse point",
            ));
        }
        if metadata.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("jsonl")
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    Ok(paths)
}

fn probe_header(
    path: &Path,
    context: &ProviderContext<'_>,
) -> DiscoveryResult<Option<HeaderProbe>> {
    let file = File::open(path).map_err(|error| DiscoveryError::io(path, error))?;
    for line in BufReader::new(file).lines() {
        context.ensure_not_cancelled()?;
        let line = line.map_err(|error| DiscoveryError::io(path, error))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            return Ok(None);
        };
        let Some(object) = value.as_object() else {
            return Ok(None);
        };
        if object.get("type").and_then(Value::as_str) != Some("session") {
            return Ok(None);
        }
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(String::from);
        let version = match object.get("version") {
            None => Some(1),
            Some(value) => value
                .as_u64()
                .and_then(|version| u32::try_from(version).ok()),
        };
        return Ok(Some(HeaderProbe { id, version }));
    }
    Ok(None)
}

fn session_id_from_filename(path: &Path) -> Option<String> {
    const UUID_LEN: usize = 36;
    let stem = path.file_stem()?.to_str()?;
    if let Some(candidate) = stem
        .len()
        .checked_sub(UUID_LEN)
        .and_then(|start| stem.get(start..))
    {
        let bytes = candidate.as_bytes();
        let is_uuid = bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
        if is_uuid {
            return Some(candidate.to_string());
        }
    }
    (!stem.trim().is_empty()).then(|| stem.to_string())
}

fn validate_native_ref(
    context: &ProviderContext<'_>,
    native: &NativeSessionRef,
) -> PiSessionResult<()> {
    if native.provider_id != PROVIDER_ID {
        return Err(PiSessionError::ProviderMismatch {
            actual: native.provider_id.clone(),
        });
    }
    let expected_root = sessions_root(context.root());
    if native.session_root.path != expected_root {
        return Err(PiSessionError::UnsafePath {
            path: native.session_root.path.clone(),
            reason: "session root does not match the configured Pi home".into(),
        });
    }
    if native
        .source_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("jsonl")
    {
        return Err(PiSessionError::UnsafePath {
            path: native.source_path.clone(),
            reason: "session path is not a JSONL file".into(),
        });
    }
    vault_io::path_safety::validate_descendant(
        &expected_root,
        &native.source_path,
        vault_io::path_safety::EntryKind::File,
        false,
        "Pi session",
    )
    .map_err(|error| PiSessionError::UnsafePath {
        path: native.source_path.clone(),
        reason: error.to_string(),
    })?;
    Ok(())
}

fn parse_session(
    path: &Path,
    contents: &str,
    context: &ProviderContext<'_>,
) -> PiSessionResult<PiSession> {
    let lines = contents.lines().collect::<Vec<_>>();
    let header_index = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .ok_or_else(|| PiSessionError::invalid_header(path, 1, "file is empty"))?;
    let header_line_number = header_index + 1;
    let header_line = lines[header_index];
    let header_value = serde_json::from_str::<Value>(header_line).map_err(|error| {
        PiSessionError::invalid_header(path, header_line_number, error.to_string())
    })?;
    let header = parse_header(
        path,
        header_line_number,
        header_value,
        header_line.to_string(),
    )?;
    if !(1..=3).contains(&header.version) {
        return Err(PiSessionError::UnsupportedVersion {
            version: header.version,
        });
    }

    let has_trailing_newline = contents.ends_with('\n');
    let mut entries = Vec::new();
    let mut nodes = Vec::new();
    let mut previous_id = None;
    let mut partial_tail = None;

    for (index, line) in lines.iter().enumerate().skip(header_index + 1) {
        ensure_not_cancelled(context)?;
        if line.trim().is_empty() {
            continue;
        }
        let line_number = index + 1;
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) if index + 1 == lines.len() && !has_trailing_newline => {
                partial_tail = Some(PiPartialTail {
                    line_number,
                    raw: (*line).to_string(),
                });
                break;
            }
            Err(error) => {
                return Err(PiSessionError::invalid_record(
                    path,
                    line_number,
                    error.to_string(),
                ));
            }
        };
        let ordinal = u64::try_from(entries.len()).map_err(|_| {
            PiSessionError::invalid_record(path, line_number, "entry ordinal overflow")
        })?;
        let entry = parse_entry(
            path,
            line_number,
            header.version,
            ordinal,
            previous_id.as_deref(),
            value,
            (*line).to_string(),
        )?;
        previous_id = Some(entry.node.id.clone());
        nodes.push(entry.node.clone());
        entries.push(entry);
    }

    let graph = BranchGraph::try_new(nodes, previous_id)?;
    Ok(PiSession {
        header,
        entries,
        graph,
        partial_tail,
    })
}

fn parse_header(
    path: &Path,
    line_number: usize,
    raw: Value,
    raw_json: String,
) -> PiSessionResult<PiSessionHeader> {
    let object = raw
        .as_object()
        .ok_or_else(|| PiSessionError::invalid_header(path, line_number, "expected an object"))?;
    if object.get("type").and_then(Value::as_str) != Some("session") {
        return Err(PiSessionError::invalid_header(
            path,
            line_number,
            "first record type must be session",
        ));
    }
    let version = match object.get("version") {
        None => 1,
        Some(value) => value
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .ok_or_else(|| {
                PiSessionError::invalid_header(path, line_number, "version must be a u32")
            })?,
    };
    let id = required_string(object, "id").ok_or_else(|| {
        PiSessionError::invalid_header(path, line_number, "id must be a non-empty string")
    })?;
    Ok(PiSessionHeader {
        version,
        id,
        timestamp: optional_string(object, "timestamp"),
        cwd: optional_string(object, "cwd"),
        parent_session: optional_string(object, "parentSession"),
        raw,
        raw_json,
    })
}

fn parse_entry(
    path: &Path,
    line_number: usize,
    version: u32,
    ordinal: u64,
    previous_id: Option<&str>,
    raw: Value,
    raw_json: String,
) -> PiSessionResult<PiSessionEntry> {
    let object = raw
        .as_object()
        .ok_or_else(|| PiSessionError::invalid_record(path, line_number, "expected an object"))?;
    let entry_type = required_string(object, "type").ok_or_else(|| {
        PiSessionError::invalid_record(path, line_number, "type must be a non-empty string")
    })?;

    let (id, parent_id) = if version == 1 {
        let id = required_string(object, "id").unwrap_or_else(|| format!("legacy:{ordinal}"));
        let parent_id = if object.contains_key("parentId") {
            nullable_string(object, "parentId")
                .map_err(|reason| PiSessionError::invalid_record(path, line_number, reason))?
        } else {
            previous_id.map(String::from)
        };
        (id, parent_id)
    } else {
        let id = required_string(object, "id").ok_or_else(|| {
            PiSessionError::invalid_record(path, line_number, "id must be a non-empty string")
        })?;
        let parent_id = nullable_string(object, "parentId")
            .map_err(|reason| PiSessionError::invalid_record(path, line_number, reason))?;
        (id, parent_id)
    };

    Ok(PiSessionEntry {
        node: BranchNode::new(id, parent_id, ordinal),
        entry_type,
        timestamp: optional_string(object, "timestamp"),
        raw,
        raw_json,
    })
}

fn required_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(String::from)
}

fn nullable_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be null or a non-empty string")),
        None => Err(format!("{key} is required")),
    }
}

fn ensure_not_cancelled(context: &ProviderContext<'_>) -> PiSessionResult<()> {
    if context.is_cancelled() {
        Err(PiSessionError::Cancelled)
    } else {
        Ok(())
    }
}

fn reject_cursor(cursor: Option<ScanCursor>) -> DiscoveryResult<()> {
    match cursor {
        Some(cursor) => Err(DiscoveryError::UnsupportedCursor(cursor)),
        None => Ok(()),
    }
}
