//! Stable provider identity and capability contracts for AgentVault.
//!
//! Provider discovery, parsing, and mutation operations are intentionally added as their
//! implementations migrate out of the application crate. This crate starts with the metadata
//! boundary that those operations share.

mod capabilities;
mod descriptor;
mod provider;

pub use capabilities::ProviderCapabilities;
pub use descriptor::{ConsistencyClass, ProviderDescriptor, SourceKind};
pub use provider::SessionProvider;
