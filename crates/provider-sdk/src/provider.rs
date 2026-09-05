use crate::{
    DetectionContext, DetectionResult, DiscoveryPage, DiscoveryResult, ProviderCapabilities,
    ProviderContext, ProviderDescriptor, ScanCursor, SessionRoot,
};

/// Base contract implemented by every in-process session provider.
///
/// Operational methods are introduced alongside provider migration so the SDK does not expose
/// placeholder canonical, backup, or recovery types before their owning crates exist.
pub trait SessionProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    fn detect(&self, context: &DetectionContext<'_>) -> DiscoveryResult<DetectionResult>;

    fn roots(&self, context: &ProviderContext<'_>) -> DiscoveryResult<Vec<SessionRoot>>;

    fn discover(
        &self,
        context: &ProviderContext<'_>,
        cursor: Option<ScanCursor>,
    ) -> DiscoveryResult<DiscoveryPage>;

    fn supports(&self, capability: ProviderCapabilities) -> bool {
        self.descriptor().supports(capability)
    }
}
