//! Stable provider identity and capability contracts for AgentVault.
//!
//! Provider operations are added as their implementations migrate out of the application crate.
//! The current boundary covers metadata, capabilities, and read-only discovery; parsing and
//! mutation contracts remain outside the SDK until their owning components are migrated.

mod capabilities;
mod descriptor;
mod discovery;
mod provider;

pub use capabilities::ProviderCapabilities;
pub use descriptor::{ConsistencyClass, ProviderDescriptor, SourceKind};
pub use discovery::{
    DetectionContext, DetectionResult, DiscoveryError, DiscoveryPage, DiscoveryResult,
    NativeSessionKind, NativeSessionRef, ProviderContext, ScanCursor, SessionRoot, SessionRootKind,
};
pub use provider::SessionProvider;
