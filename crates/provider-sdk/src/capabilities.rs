use std::ops::BitOr;

/// Explicit operations and observations a provider implementation supports.
///
/// A capability is a factual support claim, not authorization to perform the operation. Callers
/// remain responsible for policy checks, user intent, snapshots, and write-safety enforcement.
///
/// Bit positions are part of the persisted and external-provider compatibility contract. Unknown
/// bits are retained so a newer provider can be inspected without silently changing its claims.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ProviderCapabilities(u64);

impl ProviderCapabilities {
    pub const DISCOVER: Self = Self(1 << 0);
    pub const PARSE: Self = Self(1 << 1);
    pub const LIVE_HOOK: Self = Self(1 << 2);
    pub const NATIVE_RESUME: Self = Self(1 << 3);
    pub const NATIVE_FORK: Self = Self(1 << 4);
    pub const BACKUP: Self = Self(1 << 5);
    pub const RESTORE: Self = Self(1 << 6);
    pub const REPAIR_INDEX: Self = Self(1 << 7);
    pub const MOVE_CWD: Self = Self(1 << 8);
    pub const NATIVE_EXPORT: Self = Self(1 << 9);
    pub const SUBAGENT_LINEAGE: Self = Self(1 << 10);
    pub const BRANCH_GRAPH: Self = Self(1 << 11);
    pub const MANAGED_RUNTIME: Self = Self(1 << 12);
    /// The provider can mutate its native source without AgentVault's audited safety boundary.
    ///
    /// This high-risk capability is deliberately isolated from the ordinary low bits. Callers
    /// must never infer it from parsing, backup, restore, or repair support.
    pub const WRITE_NATIVE_UNSAFE: Self = Self(1 << 63);

    const KNOWN_BITS: u64 = Self::DISCOVER.0
        | Self::PARSE.0
        | Self::LIVE_HOOK.0
        | Self::NATIVE_RESUME.0
        | Self::NATIVE_FORK.0
        | Self::BACKUP.0
        | Self::RESTORE.0
        | Self::REPAIR_INDEX.0
        | Self::MOVE_CWD.0
        | Self::NATIVE_EXPORT.0
        | Self::SUBAGENT_LINEAGE.0
        | Self::BRANCH_GRAPH.0
        | Self::MANAGED_RUNTIME.0
        | Self::WRITE_NATIVE_UNSAFE.0;

    pub const fn from_bits_retain(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn has_unknown_bits(self) -> bool {
        self.0 & !Self::KNOWN_BITS != 0
    }
}

impl BitOr for ProviderCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}
