//! Read-only Codex native rollout discovery.

use std::fs;
use std::path::{Path, PathBuf};

use provider_sdk::{
    ConsistencyClass, DetectionContext, DetectionResult, DiscoveryError, DiscoveryPage,
    DiscoveryResult, NativeSessionKind, NativeSessionRef, ProviderCapabilities, ProviderContext,
    ProviderDescriptor, ScanCursor, SessionProvider, SessionRoot, SessionRootKind, SourceKind,
};

const PROVIDER_ID: &str = "codex";

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexProvider;

impl SessionProvider for CodexProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            display_name: "Codex".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            native_cli: Some("codex".into()),
            capabilities: ProviderCapabilities::DISCOVER,
            consistency_classes: vec![
                ConsistencyClass::AppendOnlyJsonl,
                ConsistencyClass::SqliteDatabase,
                ConsistencyClass::DirectoryTree,
            ],
            source_kinds: vec![SourceKind::Local, SourceKind::Wsl],
            health_probe_timeout_ms: 1_000,
        }
    }

    fn detect(&self, context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult> {
        let root = context.root();
        let metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DetectionResult::from_evidence(Vec::new()));
            }
            Err(error) => return Err(DiscoveryError::io(root, error)),
        };
        ensure_ordinary_directory(root, &metadata)?;
        let mut evidence = Vec::new();
        for entry in fs::read_dir(root).map_err(|error| DiscoveryError::io(root, error))? {
            let entry = entry.map_err(|error| DiscoveryError::io(root, error))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_marker = matches!(name.as_str(), "sessions" | "archived_sessions")
                || matches!(
                    name.as_str(),
                    "session_index.jsonl" | "thread_history.ndjson"
                )
                || (name.starts_with("state_") && name.ends_with(".sqlite"))
                || (name.starts_with("logs_") && name.ends_with(".sqlite"))
                || (name.starts_with("thread_history_") && name.ends_with(".sqlite"));
            if !is_marker {
                continue;
            }
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| DiscoveryError::io(&path, error))?;
            if vault_io::path_safety::metadata_is_link_or_reparse(&metadata) {
                return Err(DiscoveryError::unsafe_path(
                    path,
                    "detection evidence is a symlink or reparse point",
                ));
            }
            evidence.push(path);
        }
        evidence.sort();
        Ok(DetectionResult::from_evidence(evidence))
    }

    fn roots(&self, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<SessionRoot>> {
        context.ensure_not_cancelled()?;
        Ok(vec![
            SessionRoot::new(
                context.root().join("sessions"),
                SessionRootKind::Active,
                ConsistencyClass::AppendOnlyJsonl,
            ),
            SessionRoot::new(
                context.root().join("archived_sessions"),
                SessionRootKind::Archived,
                ConsistencyClass::AppendOnlyJsonl,
            ),
        ])
    }

    fn discover(
        &self,
        context: &ProviderContext<'_>,
        cursor: Option<ScanCursor>,
    ) -> DiscoveryResult<DiscoveryPage> {
        reject_cursor(cursor)?;
        context.ensure_not_cancelled()?;
        let mut sessions = Vec::new();
        for session_root in self.roots(context)? {
            sessions.extend(discover_session_root(context, session_root)?);
        }
        sessions.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(DiscoveryPage::complete(sessions))
    }
}

/// Discover one Codex rollout root for compatibility callers that already separate lifecycle
/// states. The operation reads directory metadata only and never opens or mutates a rollout.
pub fn discover_rollouts(
    context: &ProviderContext<'_>,
    kind: SessionRootKind,
) -> DiscoveryResult<Vec<NativeSessionRef>> {
    context.ensure_not_cancelled()?;
    let path = match kind {
        SessionRootKind::Active => context.root().join("sessions"),
        SessionRootKind::Archived => context.root().join("archived_sessions"),
        _ => {
            return Err(DiscoveryError::traversal(
                context.root(),
                "unsupported Codex session root kind",
            ));
        }
    };
    discover_session_root(
        context,
        SessionRoot::new(path, kind, ConsistencyClass::AppendOnlyJsonl),
    )
}

fn discover_session_root(
    context: &ProviderContext<'_>,
    session_root: SessionRoot,
) -> DiscoveryResult<Vec<NativeSessionRef>> {
    let mut sessions = Vec::new();
    for source_path in rollout_paths(&session_root.path, context)? {
        context.ensure_not_cancelled()?;
        sessions.push(NativeSessionRef::new(
            PROVIDER_ID,
            rollout_session_id(&source_path),
            source_path,
            session_root.clone(),
            NativeSessionKind::Primary,
        ));
    }
    Ok(sessions)
}

fn rollout_paths(root: &Path, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<PathBuf>> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(DiscoveryError::io(root, error)),
    };
    ensure_ordinary_directory(root, &metadata)?;
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        context.ensure_not_cancelled()?;
        let entry = entry.map_err(|error| {
            DiscoveryError::traversal(root, format!("failed to walk Codex rollouts: {error}"))
        })?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| DiscoveryError::io(entry.path(), error))?;
        if vault_io::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(DiscoveryError::unsafe_path(
                entry.path(),
                "rollout tree contains a symlink or reparse point",
            ));
        }
        let name = entry.file_name().to_string_lossy();
        if metadata.is_file() && name.starts_with("rollout-") && name.ends_with(".jsonl") {
            paths.push(entry.path().to_path_buf());
        }
    }
    Ok(paths)
}

fn ensure_ordinary_directory(path: &Path, metadata: &fs::Metadata) -> DiscoveryResult<()> {
    if !metadata.is_dir() || vault_io::path_safety::metadata_is_link_or_reparse(metadata) {
        return Err(DiscoveryError::unsafe_path(
            path,
            "session root is not an ordinary directory",
        ));
    }
    Ok(())
}

fn rollout_session_id(path: &Path) -> Option<String> {
    const UUID_LEN: usize = 36;
    let stem = path.file_stem()?.to_str()?.strip_prefix("rollout-")?;
    let candidate = stem.get(stem.len().checked_sub(UUID_LEN)?..)?;
    let bytes = candidate.as_bytes();
    let valid = bytes.iter().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => *byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    });
    valid.then(|| candidate.to_string())
}

fn reject_cursor(cursor: Option<ScanCursor>) -> DiscoveryResult<()> {
    match cursor {
        Some(cursor) => Err(DiscoveryError::UnsupportedCursor(cursor)),
        None => Ok(()),
    }
}
