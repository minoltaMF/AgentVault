use crate::{ProviderCapabilities, ProviderDescriptor};

/// Base contract implemented by every in-process session provider.
///
/// Operational methods are introduced alongside provider migration so the SDK does not expose
/// placeholder canonical, backup, or recovery types before their owning crates exist.
pub trait SessionProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    fn supports(&self, capability: ProviderCapabilities) -> bool {
        self.descriptor().supports(capability)
    }
}
