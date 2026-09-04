//! Data-safety primitives shared by AgentVault components.

pub mod atomic;
mod error;
pub mod path_safety;

pub use error::{Error, Result};
