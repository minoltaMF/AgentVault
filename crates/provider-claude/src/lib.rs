//! Read-only Claude Code native transcript discovery.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use provider_sdk::{
    ConsistencyClass, DetectionContext, DetectionResult, DiscoveryError, DiscoveryPage,
    DiscoveryResult, NativeSessionKind, NativeSessionRef, ProviderCapabilities, ProviderContext,
    ProviderDescriptor, ResumeError, ResumeOptions, ResumePlan, ResumeResult, ScanCursor,
    SessionProvider, SessionRoot, SessionRootKind, SourceKind,
};

const PROVIDER_ID: &str = "claude";

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeProvider;

impl SessionProvider for ClaudeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            display_name: "Claude Code".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            native_cli: Some("claude".into()),
            capabilities: ProviderCapabilities::DISCOVER
                | ProviderCapabilities::NATIVE_RESUME
                | ProviderCapabilities::SUBAGENT_LINEAGE,
            consistency_classes: vec![
                ConsistencyClass::AppendOnlyJsonl,
                ConsistencyClass::DirectoryTree,
            ],
            source_kinds: vec![SourceKind::Local, SourceKind::Wsl],
            health_probe_timeout_ms: 1_000,
        }
    }

    fn detect(&self, context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult> {
        let projects = projects_root(context.root());
        let evidence = match ordinary_directory(&projects)? {
            true => vec![projects],
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
        let session_root = session_root(context.root());
        let mut sessions = Vec::new();
        for path in transcript_paths(&session_root.path, context)? {
            context.ensure_not_cancelled()?;
            let is_subagent = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent-"));
            let transcript_id = transcript_session_id(&path, context)?;
            let filename_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(String::from);
            let (native_session_id, parent_native_session_id, kind) = if is_subagent {
                (filename_id, transcript_id, NativeSessionKind::Subagent)
            } else {
                (
                    transcript_id.or(filename_id),
                    None,
                    NativeSessionKind::Primary,
                )
            };
            let mut native = NativeSessionRef::new(
                PROVIDER_ID,
                native_session_id,
                path,
                session_root.clone(),
                kind,
            );
            native.parent_native_session_id = parent_native_session_id;
            sessions.push(native);
        }
        sessions.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(DiscoveryPage::complete(sessions))
    }

    fn resume_plan(
        &self,
        native: &NativeSessionRef,
        options: &ResumeOptions,
    ) -> ResumeResult<ResumePlan> {
        if native.provider_id != PROVIDER_ID {
            return Err(ResumeError::ProviderMismatch {
                expected: PROVIDER_ID.into(),
                actual: native.provider_id.clone(),
            });
        }
        if native.kind != NativeSessionKind::Primary {
            return Err(ResumeError::NativeSessionKindUnsupported {
                provider_id: PROVIDER_ID.into(),
            });
        }
        let native_id = native.native_session_id.as_ref().ok_or_else(|| {
            ResumeError::MissingNativeSessionId {
                provider_id: PROVIDER_ID.into(),
            }
        })?;
        ResumePlan::native_command(
            &self.descriptor(),
            native,
            options,
            vec![OsString::from("--resume"), OsString::from(native_id)],
        )
    }
}

fn projects_root(claude_home: &Path) -> PathBuf {
    claude_home.join("projects")
}

fn session_root(claude_home: &Path) -> SessionRoot {
    SessionRoot::new(
        projects_root(claude_home),
        SessionRootKind::Active,
        ConsistencyClass::AppendOnlyJsonl,
    )
}

fn ordinary_directory(path: &Path) -> DiscoveryResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
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

fn transcript_paths(root: &Path, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<PathBuf>> {
    if !ordinary_directory(root)? {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        context.ensure_not_cancelled()?;
        let entry = entry.map_err(|error| {
            DiscoveryError::traversal(root, format!("failed to walk Claude projects: {error}"))
        })?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| DiscoveryError::io(entry.path(), error))?;
        if vault_io::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(DiscoveryError::unsafe_path(
                entry.path(),
                "projects tree contains a symlink or reparse point",
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
    Ok(paths)
}

fn transcript_session_id(
    path: &Path,
    context: &ProviderContext<'_>,
) -> DiscoveryResult<Option<String>> {
    let file = File::open(path).map_err(|error| DiscoveryError::io(path, error))?;
    for line in BufReader::new(file).lines() {
        context.ensure_not_cancelled()?;
        let line = line.map_err(|error| DiscoveryError::io(path, error))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = value
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            return Ok(Some(id.to_string()));
        }
    }
    Ok(None)
}

fn reject_cursor(cursor: Option<ScanCursor>) -> DiscoveryResult<()> {
    match cursor {
        Some(cursor) => Err(DiscoveryError::UnsupportedCursor(cursor)),
        None => Ok(()),
    }
}
