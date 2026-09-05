use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::{NativeSessionRef, ProviderCapabilities, ProviderDescriptor};

/// Where a working-directory candidate came from.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CwdCandidateOrigin {
    SessionStart,
    ProjectLocation,
}

/// Last recorded availability of a working-directory candidate.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CwdAvailability {
    Available,
    Missing,
    Unknown,
}

/// One explicit directory that may be used to launch the provider-native CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwdCandidate {
    pub project_location_id: Option<String>,
    pub path: PathBuf,
    pub origin: CwdCandidateOrigin,
    pub availability: CwdAvailability,
}

/// User-selected terminal destination. Execution belongs to a later terminal adapter.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TerminalTarget {
    #[default]
    CurrentTerminal,
    CopyCommand,
    MacOsDefaultTerminal,
}

/// Checks an executor must complete before launching a native resume plan.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightCheck {
    ExecutableAvailable(PathBuf),
    NativeSourceExists(PathBuf),
    SelectedCwdExists(PathBuf),
}

/// Safe outcome when the native resume preflight cannot be satisfied.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResumeFallback {
    #[default]
    ReportUnavailable,
}

/// Provider-neutral inputs already approved by the cwd mapping layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeOptions {
    pub cwd_candidates: Vec<CwdCandidate>,
    pub selected_cwd: Option<PathBuf>,
    pub env_allowlist: BTreeMap<String, String>,
    pub terminal_target: TerminalTarget,
}

/// A shell-free description of how to resume one provider-native session.
///
/// This structure is inert data. Building it never starts a process or mutates a native source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePlan {
    pub provider_id: String,
    pub native_session_id: String,
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd_candidates: Vec<CwdCandidate>,
    pub selected_cwd: Option<PathBuf>,
    pub env_allowlist: BTreeMap<String, String>,
    pub terminal_target: TerminalTarget,
    pub preflight_checks: Vec<PreflightCheck>,
    pub fallback: ResumeFallback,
}

impl ResumePlan {
    /// Build an inert native command plan after checking provider identity and cwd selection.
    pub fn native_command(
        descriptor: &ProviderDescriptor,
        native: &NativeSessionRef,
        options: &ResumeOptions,
        args: Vec<OsString>,
    ) -> ResumeResult<Self> {
        if descriptor.id != native.provider_id {
            return Err(ResumeError::ProviderMismatch {
                expected: descriptor.id.clone(),
                actual: native.provider_id.clone(),
            });
        }
        if !descriptor.supports(ProviderCapabilities::NATIVE_RESUME) {
            return Err(ResumeError::NativeResumeUnsupported {
                provider_id: descriptor.id.clone(),
            });
        }
        let native_session_id = native
            .native_session_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
            .cloned()
            .ok_or_else(|| ResumeError::MissingNativeSessionId {
                provider_id: descriptor.id.clone(),
            })?;
        let executable = descriptor
            .native_cli
            .as_ref()
            .filter(|cli| !cli.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| ResumeError::MissingNativeCli {
                provider_id: descriptor.id.clone(),
            })?;
        if let Some(selected) = options.selected_cwd.as_ref() {
            if !options
                .cwd_candidates
                .iter()
                .any(|candidate| candidate.path == *selected)
            {
                return Err(ResumeError::SelectedCwdNotCandidate(selected.clone()));
            }
        }

        let mut preflight_checks = vec![
            PreflightCheck::ExecutableAvailable(executable.clone()),
            PreflightCheck::NativeSourceExists(native.source_path.clone()),
        ];
        if let Some(cwd) = options.selected_cwd.as_ref() {
            preflight_checks.push(PreflightCheck::SelectedCwdExists(cwd.clone()));
        }

        Ok(Self {
            provider_id: descriptor.id.clone(),
            native_session_id,
            executable,
            args,
            cwd_candidates: options.cwd_candidates.clone(),
            selected_cwd: options.selected_cwd.clone(),
            env_allowlist: options.env_allowlist.clone(),
            terminal_target: options.terminal_target.clone(),
            preflight_checks,
            fallback: ResumeFallback::ReportUnavailable,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    NativeResumeUnsupported { provider_id: String },
    NativeSessionKindUnsupported { provider_id: String },
    ProviderMismatch { expected: String, actual: String },
    MissingNativeSessionId { provider_id: String },
    MissingNativeCli { provider_id: String },
    SelectedCwdNotCandidate(PathBuf),
}

impl fmt::Display for ResumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeResumeUnsupported { provider_id } => {
                write!(
                    formatter,
                    "provider {provider_id} does not support native resume"
                )
            }
            Self::NativeSessionKindUnsupported { provider_id } => write!(
                formatter,
                "provider {provider_id} cannot resume this native session kind"
            ),
            Self::ProviderMismatch { expected, actual } => write!(
                formatter,
                "resume provider mismatch: expected {expected}, got {actual}"
            ),
            Self::MissingNativeSessionId { provider_id } => {
                write!(formatter, "provider {provider_id} session has no native id")
            }
            Self::MissingNativeCli { provider_id } => {
                write!(formatter, "provider {provider_id} has no native CLI")
            }
            Self::SelectedCwdNotCandidate(path) => write!(
                formatter,
                "selected cwd is not an approved candidate: {}",
                path.display()
            ),
        }
    }
}

impl Error for ResumeError {}

pub type ResumeResult<T> = Result<T, ResumeError>;
