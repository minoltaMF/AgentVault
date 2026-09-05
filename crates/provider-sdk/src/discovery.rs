use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ConsistencyClass;

/// Read-only context used to decide whether a configured provider home is present.
#[derive(Debug, Clone, Copy)]
pub struct DetectionContext<'a> {
    root: &'a Path,
}

impl<'a> DetectionContext<'a> {
    pub const fn new(root: &'a Path) -> Self {
        Self { root }
    }

    pub const fn root(&self) -> &'a Path {
        self.root
    }
}

/// Detection result with concrete filesystem evidence rather than an inferred installation claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionResult {
    pub detected: bool,
    pub evidence: Vec<PathBuf>,
}

impl DetectionResult {
    pub fn from_evidence(evidence: Vec<PathBuf>) -> Self {
        Self {
            detected: !evidence.is_empty(),
            evidence,
        }
    }
}

/// Read-only provider operation context.
#[derive(Debug, Clone, Copy)]
pub struct ProviderContext<'a> {
    root: &'a Path,
    cancellation: Option<&'a AtomicBool>,
}

impl<'a> ProviderContext<'a> {
    pub const fn new(root: &'a Path) -> Self {
        Self {
            root,
            cancellation: None,
        }
    }

    pub const fn with_cancellation(mut self, cancellation: &'a AtomicBool) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub const fn root(&self) -> &'a Path {
        self.root
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .is_some_and(|cancellation| cancellation.load(Ordering::Acquire))
    }

    pub fn ensure_not_cancelled(&self) -> DiscoveryResult<()> {
        if self.is_cancelled() {
            Err(DiscoveryError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// The lifecycle role of a native directory scanned by a provider.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRootKind {
    Active,
    Archived,
}

/// A provider-owned native session root and its consistency requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRoot {
    pub path: PathBuf,
    pub kind: SessionRootKind,
    pub consistency: ConsistencyClass,
}

impl SessionRoot {
    pub fn new(
        path: impl Into<PathBuf>,
        kind: SessionRootKind,
        consistency: ConsistencyClass,
    ) -> Self {
        Self {
            path: path.into(),
            kind,
            consistency,
        }
    }
}

/// Native transcript role known during discovery without canonical parsing.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeSessionKind {
    Primary,
    Subagent,
}

/// A read-only reference to provider-native evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSessionRef {
    pub provider_id: String,
    pub native_session_id: Option<String>,
    pub parent_native_session_id: Option<String>,
    pub source_path: PathBuf,
    pub session_root: SessionRoot,
    pub kind: NativeSessionKind,
}

impl NativeSessionRef {
    pub fn new(
        provider_id: impl Into<String>,
        native_session_id: Option<String>,
        source_path: impl Into<PathBuf>,
        session_root: SessionRoot,
        kind: NativeSessionKind,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            native_session_id,
            parent_native_session_id: None,
            source_path: source_path.into(),
            session_root,
            kind,
        }
    }

    pub fn with_parent_native_session_id(mut self, parent: impl Into<String>) -> Self {
        self.parent_native_session_id = Some(parent.into());
        self
    }
}

/// Opaque provider cursor. Cursor persistence and incremental policies belong to the registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScanCursor(String);

impl ScanCursor {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One provider discovery page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryPage {
    pub sessions: Vec<NativeSessionRef>,
    pub next_cursor: Option<ScanCursor>,
}

impl DiscoveryPage {
    pub fn complete(sessions: Vec<NativeSessionRef>) -> Self {
        Self {
            sessions,
            next_cursor: None,
        }
    }
}

#[derive(Debug)]
pub enum DiscoveryError {
    Cancelled,
    Io { path: PathBuf, source: io::Error },
    Traversal { path: PathBuf, message: String },
    UnsafePath { path: PathBuf, reason: String },
    UnsupportedCursor(ScanCursor),
}

impl DiscoveryError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn traversal(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Traversal {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn unsafe_path(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self::UnsafePath {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("provider discovery cancelled"),
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "provider discovery I/O failed at {}: {source}",
                    path.display()
                )
            }
            Self::Traversal { path, message } => write!(
                formatter,
                "provider discovery traversal failed at {}: {message}",
                path.display()
            ),
            Self::UnsafePath { path, reason } => write!(
                formatter,
                "provider discovery rejected unsafe path {}: {reason}",
                path.display()
            ),
            Self::UnsupportedCursor(cursor) => {
                write!(
                    formatter,
                    "provider does not support scan cursor: {}",
                    cursor.as_str()
                )
            }
        }
    }
}

impl Error for DiscoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type DiscoveryResult<T> = Result<T, DiscoveryError>;
