use provider_sdk::ProviderCapabilities;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRecord {
    pub id: String,
    pub display_name: String,
    pub platform: String,
    pub arch: String,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInstanceRecord {
    pub id: String,
    pub machine_id: String,
    pub provider_id: String,
    pub config_root: String,
    pub root_fingerprint: String,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub display_name: String,
    pub normalized_remotes: Vec<String>,
    pub root_commit: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSessionRecord {
    pub machine_id: String,
    pub source_instance_id: String,
    pub provider_id: String,
    pub native_session_id: String,
    pub project_id: Option<String>,
    pub root_native_session_id: Option<String>,
    pub parent_native_session_id: Option<String>,
    pub title: Option<String>,
    pub cwd_at_start: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub source_format_version: Option<String>,
    pub parser_version: String,
    pub health_status: String,
    pub resumability: String,
    pub capabilities: ProviderCapabilities,
    pub metadata: Value,
}

/// Persisted state required to resume parsing an append-only source file safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCursor {
    pub file_identity: Option<String>,
    pub size: u64,
    pub mtime_ns: i64,
    pub parsed_offset: u64,
    pub last_complete_line_offset: u64,
    pub partial_tail: Vec<u8>,
    pub parser_version: String,
    /// Hash of a provider-defined stable prefix/window used to verify the next append.
    pub last_hash: String,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObservation {
    pub file_identity: Option<String>,
    pub size: u64,
    pub mtime_ns: i64,
    /// Recomputed hash of the same stable prefix/window represented by the stored `last_hash`.
    pub verified_last_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullScanReason {
    MissingCursor,
    FileReplaced,
    Truncated,
    ParserVersionChanged,
    ContentChanged,
    IdentityUnavailable,
    HashUnverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceScanDecision {
    Unchanged,
    Resume {
        parsed_offset: u64,
        partial_tail: Vec<u8>,
    },
    FullScan(FullScanReason),
}

/// One canonical event plus enough raw provenance to rebuild or diagnose its projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEvent {
    pub event_id: String,
    pub branch_id: Option<String>,
    pub native_event_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub ordinal: i64,
    pub timestamp_ms: Option<i64>,
    pub kind: String,
    pub role: Option<String>,
    pub plain_text: Option<String>,
    pub tool_call_id: Option<String>,
    pub structured: Value,
    pub raw_byte_start: Option<u64>,
    pub raw_byte_end: Option<u64>,
    pub parse_quality: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionMode {
    Append,
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProjection {
    pub native_session_pk: i64,
    pub role: String,
    pub absolute_path: String,
    pub mode: ProjectionMode,
    /// Hash of the prior cursor's stable prefix/window, required before an append commit.
    pub verified_previous_hash: Option<String>,
    pub cursor: SourceCursor,
    pub events: Vec<CanonicalEvent>,
}
