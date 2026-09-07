//! Read-only WorkBuddy metadata overlay projection for AgentVault.
//!
//! WorkBuddy observations enrich an already-discovered Claude or Codex native session. They do
//! not create an independent transcript, discover native sessions, read WorkBuddy storage, or
//! write either product's files. Callers remain responsible for obtaining observations through a
//! public WorkBuddy boundary and for resolving the matching native session.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

const MAX_IDENTIFIER_BYTES: usize = 1_024;
const MAX_PATH_BYTES: usize = 32 * 1_024;
const MAX_SUMMARY_BYTES: usize = 64 * 1_024;
const MAX_ACTIVITY_SUMMARY_BYTES: usize = 16 * 1_024;
const MAX_ACTIVITIES_PER_OBSERVATION: usize = 10_000;

/// Native providers that WorkBuddy may enrich without becoming a session source itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeOriginProvider {
    Claude,
    Codex,
}

impl NativeOriginProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// Full native identity used to prevent overlays from crossing machines or source instances.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeSessionKey {
    machine_id: String,
    source_instance_id: String,
    native_origin_provider: NativeOriginProvider,
    native_session_id: String,
}

impl NativeSessionKey {
    pub fn new(
        machine_id: impl Into<String>,
        source_instance_id: impl Into<String>,
        native_origin_provider: NativeOriginProvider,
        native_session_id: impl Into<String>,
    ) -> OverlayResult<Self> {
        Ok(Self {
            machine_id: checked_identifier("machine_id", machine_id.into())?,
            source_instance_id: checked_identifier(
                "source_instance_id",
                source_instance_id.into(),
            )?,
            native_origin_provider,
            native_session_id: checked_identifier("native_session_id", native_session_id.into())?,
        })
    }

    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub fn source_instance_id(&self) -> &str {
        &self.source_instance_id
    }

    pub const fn native_origin_provider(&self) -> NativeOriginProvider {
        self.native_origin_provider
    }

    pub fn native_session_id(&self) -> &str {
        &self.native_session_id
    }
}

/// Public integration boundary that produced a WorkBuddy observation.
///
/// Variant order encodes deterministic tie-breaking priority for equal observation timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverlaySource {
    HookManifest,
    TranscriptProviderBridge,
    GatewayApi,
}

/// A projected value with enough provenance to explain which observation won a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedValue<T> {
    pub value: T,
    pub source: OverlaySource,
    pub observed_at_ms: i64,
}

/// Bounded WorkBuddy activity metadata. It intentionally contains no message or transcript body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkBuddyActivity {
    pub id: String,
    pub kind: String,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub occurred_at_ms: Option<i64>,
}

impl WorkBuddyActivity {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> OverlayResult<Self> {
        Ok(Self {
            id: checked_identifier("activity.id", id.into())?,
            kind: checked_identifier("activity.kind", kind.into())?,
            status: None,
            summary: None,
            occurred_at_ms: None,
        })
    }

    pub fn with_status(mut self, status: impl Into<String>) -> OverlayResult<Self> {
        self.status = Some(checked_identifier("activity.status", status.into())?);
        Ok(self)
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> OverlayResult<Self> {
        self.summary = Some(checked_text(
            "activity.summary",
            summary.into(),
            MAX_ACTIVITY_SUMMARY_BYTES,
        )?);
        Ok(self)
    }

    pub fn with_occurred_at_ms(mut self, occurred_at_ms: i64) -> Self {
        self.occurred_at_ms = Some(occurred_at_ms);
        self
    }
}

/// One caller-normalized WorkBuddy observation for an existing native session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkBuddyOverlayObservation {
    identity: NativeSessionKey,
    source: OverlaySource,
    observed_at_ms: i64,
    canonical_session_id: Option<String>,
    harness_id: Option<String>,
    transcript_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
    model: Option<String>,
    summary: Option<String>,
    activities: Vec<WorkBuddyActivity>,
}

impl WorkBuddyOverlayObservation {
    pub fn new(identity: NativeSessionKey, source: OverlaySource, observed_at_ms: i64) -> Self {
        Self {
            identity,
            source,
            observed_at_ms,
            canonical_session_id: None,
            harness_id: None,
            transcript_path: None,
            cwd: None,
            model: None,
            summary: None,
            activities: Vec::new(),
        }
    }

    pub fn with_canonical_session_id(
        mut self,
        canonical_session_id: impl Into<String>,
    ) -> OverlayResult<Self> {
        self.canonical_session_id = Some(checked_identifier(
            "canonical_session_id",
            canonical_session_id.into(),
        )?);
        Ok(self)
    }

    pub fn with_harness_id(mut self, harness_id: impl Into<String>) -> OverlayResult<Self> {
        self.harness_id = Some(checked_identifier("harness_id", harness_id.into())?);
        Ok(self)
    }

    pub fn with_transcript_path(
        mut self,
        transcript_path: impl Into<PathBuf>,
    ) -> OverlayResult<Self> {
        self.transcript_path = Some(checked_path("transcript_path", transcript_path.into())?);
        Ok(self)
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> OverlayResult<Self> {
        self.cwd = Some(checked_path("cwd", cwd.into())?);
        Ok(self)
    }

    pub fn with_model(mut self, model: impl Into<String>) -> OverlayResult<Self> {
        self.model = Some(checked_identifier("model", model.into())?);
        Ok(self)
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> OverlayResult<Self> {
        self.summary = Some(checked_text("summary", summary.into(), MAX_SUMMARY_BYTES)?);
        Ok(self)
    }

    pub fn with_activity(mut self, activity: WorkBuddyActivity) -> OverlayResult<Self> {
        if self.activities.len() >= MAX_ACTIVITIES_PER_OBSERVATION {
            return Err(OverlayError::InvalidField {
                field: "activities",
                reason: format!("contains more than {MAX_ACTIVITIES_PER_OBSERVATION} entries"),
            });
        }
        self.activities.push(checked_activity(activity)?);
        Ok(self)
    }
}

/// WorkBuddy metadata attached to exactly one native session identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkBuddySessionOverlay {
    pub identity: NativeSessionKey,
    pub canonical_session_id: Option<ObservedValue<String>>,
    pub harness_id: Option<ObservedValue<String>>,
    pub transcript_path: Option<ObservedValue<PathBuf>>,
    pub cwd: Option<ObservedValue<PathBuf>>,
    pub model: Option<ObservedValue<String>>,
    pub summary: Option<ObservedValue<String>>,
    pub activities: Vec<WorkBuddyActivity>,
    pub last_observed_at_ms: i64,
}

/// Deterministic in-memory overlay index. It performs no file, database, process, or network I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkBuddyOverlayCatalog {
    overlays: BTreeMap<NativeSessionKey, WorkBuddySessionOverlay>,
}

impl WorkBuddyOverlayCatalog {
    pub fn from_observations(
        observations: impl IntoIterator<Item = WorkBuddyOverlayObservation>,
    ) -> OverlayResult<Self> {
        let mut pending = BTreeMap::<NativeSessionKey, PendingOverlay>::new();
        for observation in observations {
            pending
                .entry(observation.identity.clone())
                .or_insert_with(|| PendingOverlay::new(observation.identity.clone()))
                .merge(observation)?;
        }

        let overlays = pending
            .into_iter()
            .map(|(key, overlay)| (key, overlay.finish()))
            .collect();
        Ok(Self { overlays })
    }

    pub fn overlay_for(&self, identity: &NativeSessionKey) -> Option<&WorkBuddySessionOverlay> {
        self.overlays.get(identity)
    }

    pub fn len(&self) -> usize {
        self.overlays.len()
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayError {
    InvalidField {
        field: &'static str,
        reason: String,
    },
    ConflictingObservation {
        field: &'static str,
        identity: NativeSessionKey,
        source: OverlaySource,
        observed_at_ms: i64,
    },
}

impl fmt::Display for OverlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid WorkBuddy overlay {field}: {reason}")
            }
            Self::ConflictingObservation {
                field,
                identity,
                source,
                observed_at_ms,
            } => write!(
                formatter,
                "conflicting WorkBuddy {field} observations for {} native session from {source:?} at {observed_at_ms}",
                identity.native_origin_provider().as_str()
            ),
        }
    }
}

impl Error for OverlayError {}

pub type OverlayResult<T> = Result<T, OverlayError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ObservationStamp {
    observed_at_ms: i64,
    source: OverlaySource,
}

#[derive(Debug)]
struct PendingOverlay {
    identity: NativeSessionKey,
    canonical_session_id: CandidateMap<String>,
    harness_id: CandidateMap<String>,
    transcript_path: CandidateMap<PathBuf>,
    cwd: CandidateMap<PathBuf>,
    model: CandidateMap<String>,
    summary: CandidateMap<String>,
    activities: BTreeMap<String, CandidateMap<WorkBuddyActivity>>,
    last_observed_at_ms: i64,
}

type CandidateMap<T> = BTreeMap<ObservationStamp, T>;

impl PendingOverlay {
    fn new(identity: NativeSessionKey) -> Self {
        Self {
            identity,
            canonical_session_id: BTreeMap::new(),
            harness_id: BTreeMap::new(),
            transcript_path: BTreeMap::new(),
            cwd: BTreeMap::new(),
            model: BTreeMap::new(),
            summary: BTreeMap::new(),
            activities: BTreeMap::new(),
            last_observed_at_ms: i64::MIN,
        }
    }

    fn merge(&mut self, observation: WorkBuddyOverlayObservation) -> OverlayResult<()> {
        let stamp = ObservationStamp {
            observed_at_ms: observation.observed_at_ms,
            source: observation.source,
        };
        insert_candidate(
            &self.identity,
            &mut self.canonical_session_id,
            stamp,
            observation.canonical_session_id,
            "canonical_session_id",
        )?;
        insert_candidate(
            &self.identity,
            &mut self.harness_id,
            stamp,
            observation.harness_id,
            "harness_id",
        )?;
        insert_candidate(
            &self.identity,
            &mut self.transcript_path,
            stamp,
            observation.transcript_path,
            "transcript_path",
        )?;
        insert_candidate(&self.identity, &mut self.cwd, stamp, observation.cwd, "cwd")?;
        insert_candidate(
            &self.identity,
            &mut self.model,
            stamp,
            observation.model,
            "model",
        )?;
        insert_candidate(
            &self.identity,
            &mut self.summary,
            stamp,
            observation.summary,
            "summary",
        )?;
        for activity in observation.activities {
            insert_candidate(
                &self.identity,
                self.activities.entry(activity.id.clone()).or_default(),
                stamp,
                Some(activity),
                "activity",
            )?;
        }
        self.last_observed_at_ms = self.last_observed_at_ms.max(observation.observed_at_ms);
        Ok(())
    }

    fn finish(self) -> WorkBuddySessionOverlay {
        let mut activities = self
            .activities
            .into_values()
            .filter_map(winning_value)
            .map(|observed| observed.value)
            .collect::<Vec<_>>();
        activities.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });

        WorkBuddySessionOverlay {
            identity: self.identity,
            canonical_session_id: winning_value(self.canonical_session_id),
            harness_id: winning_value(self.harness_id),
            transcript_path: winning_value(self.transcript_path),
            cwd: winning_value(self.cwd),
            model: winning_value(self.model),
            summary: winning_value(self.summary),
            activities,
            last_observed_at_ms: self.last_observed_at_ms,
        }
    }
}

fn insert_candidate<T: PartialEq>(
    identity: &NativeSessionKey,
    candidates: &mut CandidateMap<T>,
    stamp: ObservationStamp,
    candidate: Option<T>,
    field: &'static str,
) -> OverlayResult<()> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    if let Some(existing) = candidates.get(&stamp) {
        if existing != &candidate {
            return Err(OverlayError::ConflictingObservation {
                field,
                identity: identity.clone(),
                source: stamp.source,
                observed_at_ms: stamp.observed_at_ms,
            });
        }
        return Ok(());
    }
    candidates.insert(stamp, candidate);
    Ok(())
}

fn winning_value<T>(candidates: CandidateMap<T>) -> Option<ObservedValue<T>> {
    candidates
        .into_iter()
        .next_back()
        .map(|(stamp, value)| ObservedValue {
            value,
            source: stamp.source,
            observed_at_ms: stamp.observed_at_ms,
        })
}

fn checked_identifier(field: &'static str, value: String) -> OverlayResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid_field(field, "must not be empty"));
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(invalid_field(
            field,
            format!("exceeds {MAX_IDENTIFIER_BYTES} bytes"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid_field(field, "contains control characters"));
    }
    Ok(value.to_string())
}

fn checked_text(field: &'static str, value: String, max_bytes: usize) -> OverlayResult<String> {
    if value.trim().is_empty() {
        return Err(invalid_field(field, "must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(invalid_field(field, format!("exceeds {max_bytes} bytes")));
    }
    if value.contains('\0') {
        return Err(invalid_field(field, "contains a null character"));
    }
    Ok(value)
}

fn checked_path(field: &'static str, value: PathBuf) -> OverlayResult<PathBuf> {
    if value.as_os_str().is_empty() {
        return Err(invalid_field(field, "must not be empty"));
    }
    let display = value.to_string_lossy();
    if display.len() > MAX_PATH_BYTES {
        return Err(invalid_field(
            field,
            format!("exceeds {MAX_PATH_BYTES} display bytes"),
        ));
    }
    if display.contains('\0') {
        return Err(invalid_field(field, "contains a null character"));
    }
    Ok(value)
}

fn checked_activity(mut activity: WorkBuddyActivity) -> OverlayResult<WorkBuddyActivity> {
    activity.id = checked_identifier("activity.id", activity.id)?;
    activity.kind = checked_identifier("activity.kind", activity.kind)?;
    activity.status = activity
        .status
        .map(|status| checked_identifier("activity.status", status))
        .transpose()?;
    activity.summary = activity
        .summary
        .map(|summary| checked_text("activity.summary", summary, MAX_ACTIVITY_SUMMARY_BYTES))
        .transpose()?;
    Ok(activity)
}

fn invalid_field(field: &'static str, reason: impl Into<String>) -> OverlayError {
    OverlayError::InvalidField {
        field,
        reason: reason.into(),
    }
}
