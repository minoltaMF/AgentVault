//! Provider-neutral native resume planning for AgentVault.
//!
//! This crate maps canonical project locations to explicit cwd candidates. It does not probe or
//! launch provider executables and never mutates provider-native session data.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use provider_sdk::{
    CwdAvailability, CwdCandidate, CwdCandidateOrigin, NativeSessionRef, ResumeError,
    ResumeOptions, ResumePlan, SessionProvider, TerminalTarget,
};
use registry::ProjectLocationRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdSelection<'a> {
    Automatic,
    SessionStart,
    ProjectLocation(&'a str),
}

#[derive(Debug, Clone, Copy)]
pub struct CwdMappingRequest<'a> {
    pub current_machine_id: &'a str,
    pub project_id: Option<&'a str>,
    pub cwd_at_start: Option<&'a Path>,
    pub project_locations: &'a [ProjectLocationRecord],
    pub selection: CwdSelection<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwdMapping {
    pub candidates: Vec<CwdCandidate>,
    pub selected_cwd: Option<PathBuf>,
}

/// Map a native session's historical cwd to known paths for the same canonical project.
///
/// Automatic selection is intentionally conservative: an exact available historical path wins;
/// otherwise exactly one available current-machine location is required. Ambiguity remains
/// unselected for the caller to resolve.
pub fn map_cwd(request: CwdMappingRequest<'_>) -> Result<CwdMapping, CwdMappingError> {
    let mut candidates = Vec::new();
    if let Some(path) = request
        .cwd_at_start
        .filter(|path| !path.as_os_str().is_empty())
    {
        candidates.push(CwdCandidate {
            project_location_id: None,
            path: path.to_path_buf(),
            origin: CwdCandidateOrigin::SessionStart,
            availability: CwdAvailability::Unknown,
        });
    }

    let mut matching_locations = request
        .project_id
        .map(|project_id| {
            request
                .project_locations
                .iter()
                .filter(|location| {
                    location.project_id == project_id
                        && location.machine_id == request.current_machine_id
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    matching_locations.sort_by(|left, right| {
        right
            .last_seen_at_ms
            .cmp(&left.last_seen_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });

    for location in matching_locations {
        let path = PathBuf::from(&location.path);
        let availability = availability_from_status(&location.status);
        if let Some(existing) = candidates
            .iter_mut()
            .find(|candidate| candidate.path == path)
        {
            existing.project_location_id = Some(location.id.clone());
            existing.availability = availability;
            continue;
        }
        candidates.push(CwdCandidate {
            project_location_id: Some(location.id.clone()),
            path,
            origin: CwdCandidateOrigin::ProjectLocation,
            availability,
        });
    }

    let selected_cwd = match request.selection {
        CwdSelection::Automatic => automatic_selection(&candidates),
        CwdSelection::SessionStart => Some(
            candidates
                .iter()
                .find(|candidate| candidate.origin == CwdCandidateOrigin::SessionStart)
                .ok_or(CwdMappingError::MissingSessionStartCwd)?
                .path
                .clone(),
        ),
        CwdSelection::ProjectLocation(location_id) => {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.project_location_id.as_deref() == Some(location_id))
                .ok_or_else(|| CwdMappingError::UnknownProjectLocation(location_id.to_string()))?;
            if candidate.availability != CwdAvailability::Available {
                return Err(CwdMappingError::UnavailableProjectLocation(
                    location_id.to_string(),
                ));
            }
            Some(candidate.path.clone())
        }
    };

    Ok(CwdMapping {
        candidates,
        selected_cwd,
    })
}

/// Compose cwd mapping and a provider-owned command shape into one inert resume plan.
pub fn plan_native_resume(
    provider: &dyn SessionProvider,
    native: &NativeSessionRef,
    cwd_request: CwdMappingRequest<'_>,
    terminal_target: TerminalTarget,
) -> Result<ResumePlan, NativeResumePlanError> {
    let mapping = map_cwd(cwd_request)?;
    let options = ResumeOptions {
        cwd_candidates: mapping.candidates,
        selected_cwd: mapping.selected_cwd,
        terminal_target,
        ..ResumeOptions::default()
    };
    Ok(provider.resume_plan(native, &options)?)
}

fn automatic_selection(candidates: &[CwdCandidate]) -> Option<PathBuf> {
    if let Some(exact) = candidates.first().filter(|candidate| {
        candidate.origin == CwdCandidateOrigin::SessionStart
            && candidate.availability == CwdAvailability::Available
    }) {
        return Some(exact.path.clone());
    }
    let available = candidates
        .iter()
        .filter(|candidate| candidate.availability == CwdAvailability::Available)
        .collect::<Vec<_>>();
    (available.len() == 1).then(|| available[0].path.clone())
}

fn availability_from_status(status: &str) -> CwdAvailability {
    if status.trim().eq_ignore_ascii_case("available") {
        CwdAvailability::Available
    } else if status.trim().eq_ignore_ascii_case("missing") {
        CwdAvailability::Missing
    } else {
        CwdAvailability::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdMappingError {
    MissingSessionStartCwd,
    UnknownProjectLocation(String),
    UnavailableProjectLocation(String),
}

impl fmt::Display for CwdMappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSessionStartCwd => {
                formatter.write_str("native session has no recorded start cwd")
            }
            Self::UnknownProjectLocation(id) => {
                write!(
                    formatter,
                    "project location is not eligible for this resume: {id}"
                )
            }
            Self::UnavailableProjectLocation(id) => {
                write!(formatter, "project location is not available: {id}")
            }
        }
    }
}

impl Error for CwdMappingError {}

#[derive(Debug)]
pub enum NativeResumePlanError {
    CwdMapping(CwdMappingError),
    Provider(ResumeError),
}

impl fmt::Display for NativeResumePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CwdMapping(source) => write!(formatter, "cwd mapping failed: {source}"),
            Self::Provider(source) => {
                write!(formatter, "provider resume planning failed: {source}")
            }
        }
    }
}

impl Error for NativeResumePlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CwdMapping(source) => Some(source),
            Self::Provider(source) => Some(source),
        }
    }
}

impl From<CwdMappingError> for NativeResumePlanError {
    fn from(source: CwdMappingError) -> Self {
        Self::CwdMapping(source)
    }
}

impl From<ResumeError> for NativeResumePlanError {
    fn from(source: ResumeError) -> Self {
        Self::Provider(source)
    }
}
