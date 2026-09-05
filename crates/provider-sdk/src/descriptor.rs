use crate::ProviderCapabilities;

/// How a provider's native evidence must be captured and checked for consistency.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsistencyClass {
    AppendOnlyJsonl,
    AtomicReplaceFile,
    SqliteDatabase,
    DirectoryTree,
    ExternalReference,
}

/// The environment through which native provider evidence is reached.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Local,
    Wsl,
    Ssh,
    ExternalProcess,
}

/// Provider metadata and explicit capability claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub native_cli: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub consistency_classes: Vec<ConsistencyClass>,
    pub source_kinds: Vec<SourceKind>,
    pub health_probe_timeout_ms: u64,
}

impl ProviderDescriptor {
    pub const fn supports(&self, capability: ProviderCapabilities) -> bool {
        self.capabilities.contains(capability)
    }
}
