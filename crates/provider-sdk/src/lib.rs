//! Stable provider identity and capability contracts for AgentVault.
//!
//! Provider operations are added as their implementations migrate out of the application crate.
//! The current boundary covers metadata, capabilities, read-only discovery, provider-neutral
//! branch graphs, and inert native-resume plans. Canonical mutation contracts remain outside the
//! SDK until their owning components are migrated.

mod branch_graph;
mod capabilities;
mod descriptor;
mod discovery;
mod provider;
mod resume;

pub use branch_graph::{BranchGraph, BranchGraphError, BranchNode};
pub use capabilities::ProviderCapabilities;
pub use descriptor::{ConsistencyClass, ProviderDescriptor, SourceKind};
pub use discovery::{
    DetectionContext, DetectionResult, DiscoveryError, DiscoveryPage, DiscoveryResult,
    NativeSessionKind, NativeSessionRef, ProviderContext, ScanCursor, SessionRoot, SessionRootKind,
};
pub use provider::SessionProvider;
pub use resume::{
    CwdAvailability, CwdCandidate, CwdCandidateOrigin, PreflightCheck, ResumeError, ResumeFallback,
    ResumeOptions, ResumePlan, ResumeResult, TerminalTarget,
};
