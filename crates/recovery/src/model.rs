use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

pub type RecoveryCapsuleResult<T> = Result<T, RecoveryCapsuleError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryCapsuleError {
    #[error("recovery capsule field must not be empty: {0}")]
    EmptyField(&'static str),
    #[error("recovery capsule field contains an unsupported control character: {0}")]
    ControlCharacter(&'static str),
    #[error("invalid recovery evidence URI: {0}")]
    InvalidEvidenceUri(String),
    #[error("multiple recovery events use the same evidence URI: {0}")]
    DuplicateEventEvidence(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceQuality {
    NativeStructured,
    VerifiedSnapshot,
    UnverifiedSnapshot,
    TerminalTranscript,
    MetadataOnly,
}

impl EvidenceQuality {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NativeStructured => "native_structured",
            Self::VerifiedSnapshot => "verified_snapshot",
            Self::UnverifiedSnapshot => "unverified_snapshot",
            Self::TerminalTranscript => "terminal_transcript",
            Self::MetadataOnly => "metadata_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceRef(String);

impl EvidenceRef {
    pub fn try_new(value: impl Into<String>) -> RecoveryCapsuleResult<Self> {
        let value = value.into();
        let valid_scheme = ["event://", "snapshot://", "file://"].iter().any(|scheme| {
            value
                .strip_prefix(scheme)
                .is_some_and(|rest| !rest.is_empty())
        });
        if !valid_scheme
            || value
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(RecoveryCapsuleError::InvalidEvidenceUri(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryIdentityInput {
    pub provider_id: String,
    pub native_session_id: String,
    pub machine_id: String,
    pub source_instance_id: String,
    pub work_session_id: Option<String>,
    pub project_id: Option<String>,
    pub original_cwd: Option<String>,
    pub current_mapped_cwd: Option<String>,
    pub snapshot_id: Option<String>,
    pub captured_at: DateTime<Utc>,
    pub evidence_quality: EvidenceQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEventKind {
    UserMessage,
    AssistantOutput { complete: bool },
    ProviderSummary,
    ToolFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryEventInput {
    pub ordinal: i64,
    pub kind: RecoveryEventKind,
    pub text: String,
    pub evidence: EvidenceRef,
}

impl RecoveryEventInput {
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryNoteInput {
    pub ordinal: i64,
    pub text: String,
    pub evidence: Option<EvidenceRef>,
}

impl RecoveryNoteInput {
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryGitInput {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub diff_stat: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryArtifactInput {
    pub label: String,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCheckpointInput {
    pub checkpoint_id: String,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCapsuleDraft {
    pub identity: RecoveryIdentityInput,
    pub events: Vec<RecoveryEventInput>,
    pub current_state: Option<String>,
    pub decisions: Vec<RecoveryNoteInput>,
    pub completed: Vec<RecoveryNoteInput>,
    pub open_problems: Vec<RecoveryNoteInput>,
    pub todos: Vec<RecoveryNoteInput>,
    pub recommended_actions: Vec<RecoveryNoteInput>,
    pub git: Option<RecoveryGitInput>,
    pub artifacts: Vec<RecoveryArtifactInput>,
    pub last_successful_checkpoint: Option<RecoveryCheckpointInput>,
    pub evidence: Vec<EvidenceRef>,
}

impl RecoveryCapsuleDraft {
    pub fn new(identity: RecoveryIdentityInput) -> Self {
        Self {
            identity,
            events: Vec::new(),
            current_state: None,
            decisions: Vec::new(),
            completed: Vec::new(),
            open_problems: Vec::new(),
            todos: Vec::new(),
            recommended_actions: Vec::new(),
            git: None,
            artifacts: Vec::new(),
            last_successful_checkpoint: None,
            evidence: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCapsule {
    identity: RecoveryIdentityInput,
    goal: Option<RecoveryEventInput>,
    recent_user_messages: Vec<RecoveryEventInput>,
    last_complete_assistant_output: Option<RecoveryEventInput>,
    provider_summary: Option<RecoveryEventInput>,
    recent_failures: Vec<RecoveryEventInput>,
    current_state: Option<String>,
    decisions: Vec<RecoveryNoteInput>,
    completed: Vec<RecoveryNoteInput>,
    open_problems: Vec<RecoveryNoteInput>,
    todos: Vec<RecoveryNoteInput>,
    recommended_actions: Vec<RecoveryNoteInput>,
    git: Option<RecoveryGitInput>,
    artifacts: Vec<RecoveryArtifactInput>,
    last_successful_checkpoint: Option<RecoveryCheckpointInput>,
    evidence: Vec<EvidenceRef>,
}

impl RecoveryCapsule {
    pub fn try_new(mut draft: RecoveryCapsuleDraft) -> RecoveryCapsuleResult<Self> {
        normalize_inline("provider_id", &mut draft.identity.provider_id)?;
        normalize_inline("native_session_id", &mut draft.identity.native_session_id)?;
        normalize_inline("machine_id", &mut draft.identity.machine_id)?;
        normalize_inline("source_instance_id", &mut draft.identity.source_instance_id)?;
        normalize_optional_inline("work_session_id", &mut draft.identity.work_session_id)?;
        normalize_optional_inline("project_id", &mut draft.identity.project_id)?;
        normalize_optional_inline("original_cwd", &mut draft.identity.original_cwd)?;
        normalize_optional_inline("current_mapped_cwd", &mut draft.identity.current_mapped_cwd)?;
        normalize_optional_inline("snapshot_id", &mut draft.identity.snapshot_id)?;
        normalize_optional_block("current_state", &mut draft.current_state)?;
        canonicalize_notes("decision.text", &mut draft.decisions)?;
        canonicalize_notes("completed.text", &mut draft.completed)?;
        canonicalize_notes("open_problem.text", &mut draft.open_problems)?;
        canonicalize_notes("todo.text", &mut draft.todos)?;
        canonicalize_notes("recommended_action.text", &mut draft.recommended_actions)?;
        if let Some(git) = &mut draft.git {
            normalize_optional_inline("git.branch", &mut git.branch)?;
            normalize_optional_inline("git.head", &mut git.head)?;
            canonicalize_inline_values("git.dirty_file", &mut git.dirty_files)?;
            canonicalize_inline_values("git.changed_file", &mut git.changed_files)?;
            normalize_optional_block("git.diff_stat", &mut git.diff_stat)?;
        }
        for artifact in &mut draft.artifacts {
            normalize_inline("artifact.label", &mut artifact.label)?;
        }
        draft.artifacts.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.evidence.cmp(&right.evidence))
        });
        draft.artifacts.dedup();
        if let Some(checkpoint) = &mut draft.last_successful_checkpoint {
            normalize_inline("checkpoint_id", &mut checkpoint.checkpoint_id)?;
        }
        let mut event_evidence = BTreeSet::new();
        for event in &mut draft.events {
            normalize_block("event.text", &mut event.text)?;
            if !event_evidence.insert(event.evidence.clone()) {
                return Err(RecoveryCapsuleError::DuplicateEventEvidence(
                    event.evidence.as_str().to_string(),
                ));
            }
        }
        draft.events.sort_by(|left, right| {
            left.ordinal
                .cmp(&right.ordinal)
                .then_with(|| left.evidence.cmp(&right.evidence))
        });
        let user_messages = draft
            .events
            .iter()
            .filter(|event| event.kind == RecoveryEventKind::UserMessage)
            .cloned()
            .collect::<Vec<_>>();
        let goal = user_messages.first().cloned();
        let recent_user_messages: Vec<RecoveryEventInput> = user_messages
            .into_iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let last_complete_assistant_output = draft
            .events
            .iter()
            .rev()
            .find(|event| event.kind == RecoveryEventKind::AssistantOutput { complete: true })
            .cloned();
        let provider_summary = draft
            .events
            .iter()
            .rev()
            .find(|event| event.kind == RecoveryEventKind::ProviderSummary)
            .cloned();
        let recent_failures: Vec<RecoveryEventInput> = draft
            .events
            .iter()
            .filter(|event| event.kind == RecoveryEventKind::ToolFailure)
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut evidence = draft.evidence.into_iter().collect::<BTreeSet<_>>();
        evidence.extend(
            goal.iter()
                .chain(recent_user_messages.iter())
                .chain(last_complete_assistant_output.iter())
                .chain(provider_summary.iter())
                .chain(recent_failures.iter())
                .map(|event| event.evidence.clone()),
        );
        for notes in [
            &draft.decisions,
            &draft.completed,
            &draft.open_problems,
            &draft.todos,
            &draft.recommended_actions,
        ] {
            evidence.extend(notes.iter().filter_map(|note| note.evidence.clone()));
        }
        evidence.extend(
            draft
                .artifacts
                .iter()
                .map(|artifact| artifact.evidence.clone()),
        );
        if let Some(checkpoint) = &draft.last_successful_checkpoint {
            evidence.insert(checkpoint.evidence.clone());
        }
        if let Some(snapshot_id) = &draft.identity.snapshot_id {
            evidence.insert(EvidenceRef::try_new(format!("snapshot://{snapshot_id}"))?);
        }

        Ok(Self {
            identity: draft.identity,
            goal,
            recent_user_messages,
            last_complete_assistant_output,
            provider_summary,
            recent_failures,
            current_state: draft.current_state,
            decisions: draft.decisions,
            completed: draft.completed,
            open_problems: draft.open_problems,
            todos: draft.todos,
            recommended_actions: draft.recommended_actions,
            git: draft.git,
            artifacts: draft.artifacts,
            last_successful_checkpoint: draft.last_successful_checkpoint,
            evidence: evidence.into_iter().collect(),
        })
    }

    pub const fn identity(&self) -> &RecoveryIdentityInput {
        &self.identity
    }

    pub const fn goal(&self) -> Option<&RecoveryEventInput> {
        self.goal.as_ref()
    }

    pub fn recent_user_messages(&self) -> &[RecoveryEventInput] {
        &self.recent_user_messages
    }

    pub fn last_valid_user_request(&self) -> Option<&RecoveryEventInput> {
        self.recent_user_messages.last()
    }

    pub const fn last_complete_assistant_output(&self) -> Option<&RecoveryEventInput> {
        self.last_complete_assistant_output.as_ref()
    }

    pub const fn provider_summary(&self) -> Option<&RecoveryEventInput> {
        self.provider_summary.as_ref()
    }

    pub fn recent_failures(&self) -> &[RecoveryEventInput] {
        &self.recent_failures
    }

    pub fn current_state(&self) -> Option<&str> {
        self.current_state.as_deref()
    }

    pub fn decisions(&self) -> &[RecoveryNoteInput] {
        &self.decisions
    }

    pub fn completed(&self) -> &[RecoveryNoteInput] {
        &self.completed
    }

    pub fn open_problems(&self) -> &[RecoveryNoteInput] {
        &self.open_problems
    }

    pub fn todos(&self) -> &[RecoveryNoteInput] {
        &self.todos
    }

    pub fn recommended_actions(&self) -> &[RecoveryNoteInput] {
        &self.recommended_actions
    }

    pub const fn git(&self) -> Option<&RecoveryGitInput> {
        self.git.as_ref()
    }

    pub fn artifacts(&self) -> &[RecoveryArtifactInput] {
        &self.artifacts
    }

    pub const fn last_successful_checkpoint(&self) -> Option<&RecoveryCheckpointInput> {
        self.last_successful_checkpoint.as_ref()
    }

    pub fn evidence(&self) -> &[EvidenceRef] {
        &self.evidence
    }
}

fn normalize_inline(field: &'static str, value: &mut String) -> RecoveryCapsuleResult<()> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(RecoveryCapsuleError::EmptyField(field));
    }
    if normalized.chars().any(char::is_control) {
        return Err(RecoveryCapsuleError::ControlCharacter(field));
    }
    *value = normalized.to_string();
    Ok(())
}

fn normalize_block(field: &'static str, value: &mut String) -> RecoveryCapsuleResult<()> {
    let line_endings = value.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = line_endings.trim();
    if normalized.is_empty() {
        return Err(RecoveryCapsuleError::EmptyField(field));
    }
    if normalized
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(RecoveryCapsuleError::ControlCharacter(field));
    }
    *value = normalized.to_string();
    Ok(())
}

fn normalize_optional_inline(
    field: &'static str,
    value: &mut Option<String>,
) -> RecoveryCapsuleResult<()> {
    if let Some(value) = value {
        normalize_inline(field, value)?;
    }
    Ok(())
}

fn normalize_optional_block(
    field: &'static str,
    value: &mut Option<String>,
) -> RecoveryCapsuleResult<()> {
    if let Some(value) = value {
        normalize_block(field, value)?;
    }
    Ok(())
}

fn canonicalize_notes(
    field: &'static str,
    notes: &mut Vec<RecoveryNoteInput>,
) -> RecoveryCapsuleResult<()> {
    for note in notes.iter_mut() {
        normalize_block(field, &mut note.text)?;
    }
    notes.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.evidence.cmp(&right.evidence))
            .then_with(|| left.text.cmp(&right.text))
    });
    notes.dedup();
    Ok(())
}

fn canonicalize_inline_values(
    field: &'static str,
    values: &mut Vec<String>,
) -> RecoveryCapsuleResult<()> {
    for value in values.iter_mut() {
        normalize_inline(field, value)?;
    }
    values.sort();
    values.dedup();
    Ok(())
}
