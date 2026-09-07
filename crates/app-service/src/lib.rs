//! Application-level projections for AgentVault.
//!
//! This crate currently builds an in-memory Recovery Inbox from caller-owned facts and Provider
//! Doctor reports. It does not persist events or perform repair, restore, or native-session writes.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;

use health::{
    DiagnosticSeverity, ProviderDiagnostic, ProviderDiagnosticCode, ProviderDoctorReport,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoveryUrgency {
    Error,
    Warning,
    Info,
}

impl RecoveryUrgency {
    pub const fn is_actionable(self) -> bool {
        matches!(self, Self::Error | Self::Warning)
    }
}

impl From<DiagnosticSeverity> for RecoveryUrgency {
    fn from(value: DiagnosticSeverity) -> Self {
        match value {
            DiagnosticSeverity::Error => Self::Error,
            DiagnosticSeverity::Warning => Self::Warning,
            DiagnosticSeverity::Info => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoveryInboxKind {
    RestoreConflict,
    ActiveSessionWithoutCheckpoint,
    NativeIndexInvisible,
    CleanupDeadlineNear,
    ProjectLocationMoved,
    ProviderHealth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySignal {
    pub key: String,
    pub kind: RecoveryInboxKind,
    pub urgency: RecoveryUrgency,
    pub observed_at_ms: i64,
    pub title: String,
    pub recommended_action: String,
}

impl RecoverySignal {
    pub fn new(
        key: impl Into<String>,
        kind: RecoveryInboxKind,
        urgency: RecoveryUrgency,
        observed_at_ms: i64,
        title: impl Into<String>,
        recommended_action: impl Into<String>,
    ) -> Result<Self, RecoverySignalError> {
        let key = key.into().trim().to_owned();
        let title = normalize_display_text(&title.into());
        let recommended_action = normalize_display_text(&recommended_action.into());
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(RecoverySignalError::InvalidKey);
        }
        if title.is_empty() {
            return Err(RecoverySignalError::EmptyTitle);
        }
        if recommended_action.is_empty() {
            return Err(RecoverySignalError::EmptyRecommendedAction);
        }
        Ok(Self {
            key,
            kind,
            urgency,
            observed_at_ms,
            title,
            recommended_action,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverySignalError {
    InvalidKey,
    EmptyTitle,
    EmptyRecommendedAction,
}

impl fmt::Display for RecoverySignalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidKey => "recovery signal key must be non-empty and contain no controls",
            Self::EmptyTitle => "recovery signal title must not be empty",
            Self::EmptyRecommendedAction => "recovery signal recommended action must not be empty",
        })
    }
}

impl Error for RecoverySignalError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryInbox {
    pub items: Vec<RecoverySignal>,
    pub error_count: usize,
    pub warning_count: usize,
}

pub fn build_recovery_inbox<'a, S, R>(signals: S, provider_reports: R) -> RecoveryInbox
where
    S: IntoIterator<Item = RecoverySignal>,
    R: IntoIterator<Item = &'a ProviderDoctorReport>,
{
    let projected = provider_reports
        .into_iter()
        .flat_map(provider_signals)
        .chain(signals)
        .filter(|signal| signal.urgency.is_actionable());
    let mut by_key = BTreeMap::new();
    for signal in projected {
        match by_key.entry(signal.key.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(signal);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if signal_is_preferred(&signal, entry.get()) {
                    entry.insert(signal);
                }
            }
        }
    }
    let mut items = by_key.into_values().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        left.urgency
            .cmp(&right.urgency)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.observed_at_ms.cmp(&left.observed_at_ms))
            .then_with(|| left.key.cmp(&right.key))
    });
    let error_count = items
        .iter()
        .filter(|item| item.urgency == RecoveryUrgency::Error)
        .count();
    let warning_count = items
        .iter()
        .filter(|item| item.urgency == RecoveryUrgency::Warning)
        .count();
    RecoveryInbox {
        items,
        error_count,
        warning_count,
    }
}

fn provider_signals(report: &ProviderDoctorReport) -> impl Iterator<Item = RecoverySignal> + '_ {
    report
        .diagnostics
        .iter()
        .map(move |diagnostic| provider_signal(report, diagnostic))
}

fn provider_signal(
    report: &ProviderDoctorReport,
    diagnostic: &ProviderDiagnostic,
) -> RecoverySignal {
    let kind = match diagnostic.code {
        ProviderDiagnosticCode::ActiveSessionWithoutCheckpoint => {
            RecoveryInboxKind::ActiveSessionWithoutCheckpoint
        }
        ProviderDiagnosticCode::CleanupDeadlineNear => RecoveryInboxKind::CleanupDeadlineNear,
        ProviderDiagnosticCode::NativeIndexInvisible => RecoveryInboxKind::NativeIndexInvisible,
        ProviderDiagnosticCode::ProjectLocationMoved => RecoveryInboxKind::ProjectLocationMoved,
        ProviderDiagnosticCode::RestoreConflict => RecoveryInboxKind::RestoreConflict,
        _ => RecoveryInboxKind::ProviderHealth,
    };
    RecoverySignal::new(
        format!(
            "provider:{}:{}",
            encode_key_component(&report.provider_id),
            diagnostic.code.as_str()
        ),
        kind,
        diagnostic.severity.into(),
        report.observed_at_ms,
        non_empty_text(&diagnostic.summary, "Provider health issue"),
        non_empty_text(
            &diagnostic.recommended_action,
            "Inspect the provider report before taking action.",
        ),
    )
    .expect("encoded provider diagnostics always form a valid recovery signal")
}

fn signal_is_preferred(candidate: &RecoverySignal, current: &RecoverySignal) -> bool {
    candidate.urgency < current.urgency
        || (candidate.urgency == current.urgency
            && (candidate.observed_at_ms > current.observed_at_ms
                || (candidate.observed_at_ms == current.observed_at_ms
                    && (
                        candidate.kind,
                        &candidate.title,
                        &candidate.recommended_action,
                    ) < (current.kind, &current.title, &current.recommended_action))))
}

fn normalize_display_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty_text(value: &str, fallback: &str) -> String {
    let value = normalize_display_text(value);
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

fn encode_key_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
