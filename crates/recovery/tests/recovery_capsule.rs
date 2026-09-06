use chrono::{TimeZone, Utc};
use recovery::{
    EvidenceQuality, EvidenceRef, RecoveryArtifactInput, RecoveryCapsule, RecoveryCapsuleDraft,
    RecoveryCapsuleError, RecoveryCheckpointInput, RecoveryEventInput, RecoveryEventKind,
    RecoveryGitInput, RecoveryIdentityInput, RecoveryNoteInput, RECOVERY_CAPSULE_FORMAT,
};

fn evidence(uri: &str) -> EvidenceRef {
    EvidenceRef::try_new(uri).expect("valid evidence URI")
}

fn identity() -> RecoveryIdentityInput {
    RecoveryIdentityInput {
        provider_id: "claude".to_string(),
        native_session_id: "native-session".to_string(),
        machine_id: "machine-local".to_string(),
        source_instance_id: "claude-home-default".to_string(),
        work_session_id: Some("work-session".to_string()),
        project_id: Some("agentvault".to_string()),
        original_cwd: Some("/workspace/old".to_string()),
        current_mapped_cwd: Some("/workspace/current".to_string()),
        snapshot_id: Some("snap-001".to_string()),
        captured_at: Utc
            .with_ymd_and_hms(2026, 9, 7, 8, 30, 0)
            .single()
            .expect("valid timestamp"),
        evidence_quality: EvidenceQuality::VerifiedSnapshot,
    }
}

fn event(
    ordinal: i64,
    kind: RecoveryEventKind,
    text: &str,
    evidence_uri: &str,
) -> RecoveryEventInput {
    RecoveryEventInput {
        ordinal,
        kind,
        text: text.to_string(),
        evidence: evidence(evidence_uri),
    }
}

#[test]
fn event_extraction_is_chronological_and_ignores_incomplete_assistant_output() {
    let mut draft = RecoveryCapsuleDraft::new(identity());
    draft.events = vec![
        event(
            40,
            RecoveryEventKind::UserMessage,
            "fourth request",
            "event://claude/native-session/user-4",
        ),
        event(
            10,
            RecoveryEventKind::UserMessage,
            "original goal",
            "event://claude/native-session/user-1",
        ),
        event(
            60,
            RecoveryEventKind::AssistantOutput { complete: false },
            "partial output",
            "event://claude/native-session/assistant-partial",
        ),
        event(
            20,
            RecoveryEventKind::UserMessage,
            "second request",
            "event://claude/native-session/user-2",
        ),
        event(
            50,
            RecoveryEventKind::AssistantOutput { complete: true },
            "last complete output",
            "event://claude/native-session/assistant-complete",
        ),
        event(
            30,
            RecoveryEventKind::UserMessage,
            "third request",
            "event://claude/native-session/user-3",
        ),
    ];

    let capsule = RecoveryCapsule::try_new(draft).expect("recovery capsule");

    assert_eq!(capsule.goal().expect("goal").text(), "original goal");
    assert_eq!(
        capsule
            .recent_user_messages()
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>(),
        vec!["second request", "third request", "fourth request"]
    );
    assert_eq!(
        capsule
            .last_valid_user_request()
            .expect("last user request")
            .text(),
        "fourth request"
    );
    assert_eq!(
        capsule
            .last_complete_assistant_output()
            .expect("complete assistant output")
            .text(),
        "last complete output"
    );
}

#[test]
fn equivalent_event_sets_produce_the_same_capsule_regardless_of_input_order() {
    let events = vec![
        event(
            10,
            RecoveryEventKind::UserMessage,
            "second stable key",
            "event://claude/native-session/user-b",
        ),
        event(
            10,
            RecoveryEventKind::UserMessage,
            "first stable key",
            "event://claude/native-session/user-a",
        ),
        event(
            20,
            RecoveryEventKind::AssistantOutput { complete: true },
            "complete output",
            "event://claude/native-session/assistant",
        ),
    ];
    let mut forward = RecoveryCapsuleDraft::new(identity());
    forward.events = events.clone();
    let mut reversed = RecoveryCapsuleDraft::new(identity());
    reversed.events = events.into_iter().rev().collect();

    let forward = RecoveryCapsule::try_new(forward).expect("forward capsule");
    let reversed = RecoveryCapsule::try_new(reversed).expect("reversed capsule");

    assert_eq!(forward, reversed);
}

#[test]
fn extraction_keeps_the_latest_provider_summary_and_three_recent_failures() {
    let mut draft = RecoveryCapsuleDraft::new(identity());
    draft.events = vec![
        event(
            5,
            RecoveryEventKind::ProviderSummary,
            "old summary",
            "event://claude/native-session/summary-old",
        ),
        event(
            40,
            RecoveryEventKind::ToolFailure,
            "failure four",
            "event://claude/native-session/failure-4",
        ),
        event(
            10,
            RecoveryEventKind::ToolFailure,
            "failure one",
            "event://claude/native-session/failure-1",
        ),
        event(
            25,
            RecoveryEventKind::ProviderSummary,
            "current summary",
            "event://claude/native-session/summary-current",
        ),
        event(
            30,
            RecoveryEventKind::ToolFailure,
            "failure three",
            "event://claude/native-session/failure-3",
        ),
        event(
            20,
            RecoveryEventKind::ToolFailure,
            "failure two",
            "event://claude/native-session/failure-2",
        ),
    ];

    let capsule = RecoveryCapsule::try_new(draft).expect("recovery capsule");

    assert_eq!(
        capsule.provider_summary().expect("provider summary").text(),
        "current summary"
    );
    assert_eq!(
        capsule
            .recent_failures()
            .iter()
            .map(|failure| failure.text())
            .collect::<Vec<_>>(),
        vec!["failure two", "failure three", "failure four"]
    );
}

#[test]
fn evidence_references_accept_only_explicit_local_provenance_schemes() {
    for uri in [
        "event://claude/native-session/event-1",
        "snapshot://snap-001",
        "file://workspace/src/lib.rs",
    ] {
        assert_eq!(evidence(uri).as_str(), uri);
    }

    for uri in [
        "https://example.com/session",
        "event://",
        "snapshot://snap 001",
        "file://bad\npath",
        "event://bad\0id",
    ] {
        assert_eq!(
            EvidenceRef::try_new(uri).expect_err("invalid evidence URI"),
            RecoveryCapsuleError::InvalidEvidenceUri(uri.to_string())
        );
    }
}

#[test]
fn required_identity_fields_are_trimmed_and_must_not_be_empty() {
    let mut normalized_identity = identity();
    normalized_identity.provider_id = "  claude  ".to_string();
    let normalized = RecoveryCapsule::try_new(RecoveryCapsuleDraft::new(normalized_identity))
        .expect("normalized identity");
    assert_eq!(normalized.identity().provider_id, "claude");

    let mut invalid_identity = identity();
    invalid_identity.native_session_id = " \r\n ".to_string();
    assert_eq!(
        RecoveryCapsule::try_new(RecoveryCapsuleDraft::new(invalid_identity))
            .expect_err("empty native session ID"),
        RecoveryCapsuleError::EmptyField("native_session_id")
    );

    let mut multiline_identity = identity();
    multiline_identity.machine_id = "machine\nother".to_string();
    assert_eq!(
        RecoveryCapsule::try_new(RecoveryCapsuleDraft::new(multiline_identity))
            .expect_err("multiline machine ID"),
        RecoveryCapsuleError::ControlCharacter("machine_id")
    );
}

#[test]
fn event_text_is_normalized_and_rejects_empty_or_unsafe_content() {
    let mut normalized_draft = RecoveryCapsuleDraft::new(identity());
    normalized_draft.events = vec![event(
        1,
        RecoveryEventKind::UserMessage,
        "  first line\r\nsecond line\rthird line  ",
        "event://claude/native-session/normalized",
    )];
    let normalized = RecoveryCapsule::try_new(normalized_draft).expect("normalized event text");
    assert_eq!(
        normalized.goal().expect("goal").text(),
        "first line\nsecond line\nthird line"
    );

    let mut empty_draft = RecoveryCapsuleDraft::new(identity());
    empty_draft.events = vec![event(
        1,
        RecoveryEventKind::UserMessage,
        " \r\n ",
        "event://claude/native-session/empty",
    )];
    assert_eq!(
        RecoveryCapsule::try_new(empty_draft).expect_err("empty event text"),
        RecoveryCapsuleError::EmptyField("event.text")
    );

    let mut unsafe_draft = RecoveryCapsuleDraft::new(identity());
    unsafe_draft.events = vec![event(
        1,
        RecoveryEventKind::UserMessage,
        "unsafe\0text",
        "event://claude/native-session/unsafe",
    )];
    assert_eq!(
        RecoveryCapsule::try_new(unsafe_draft).expect_err("unsafe event text"),
        RecoveryCapsuleError::ControlCharacter("event.text")
    );
}

#[test]
fn duplicate_event_provenance_is_rejected() {
    let duplicate = "event://claude/native-session/duplicate";
    let mut draft = RecoveryCapsuleDraft::new(identity());
    draft.events = vec![
        event(1, RecoveryEventKind::UserMessage, "goal", duplicate),
        event(
            2,
            RecoveryEventKind::AssistantOutput { complete: true },
            "answer",
            duplicate,
        ),
    ];

    assert_eq!(
        RecoveryCapsule::try_new(draft).expect_err("duplicate event evidence"),
        RecoveryCapsuleError::DuplicateEventEvidence(duplicate.to_string())
    );
}

#[test]
fn structured_context_and_evidence_are_canonicalized() {
    let completed_evidence = evidence("event://claude/native-session/completed");
    let first_completed = RecoveryNoteInput {
        ordinal: 10,
        text: "first completed item".to_string(),
        evidence: Some(completed_evidence.clone()),
    };
    let mut draft = RecoveryCapsuleDraft::new(identity());
    draft.events = vec![event(
        1,
        RecoveryEventKind::UserMessage,
        "goal",
        "event://claude/native-session/goal",
    )];
    draft.completed = vec![
        RecoveryNoteInput {
            ordinal: 20,
            text: "second completed item".to_string(),
            evidence: None,
        },
        first_completed.clone(),
        first_completed,
    ];
    draft.git = Some(RecoveryGitInput {
        branch: Some("main".to_string()),
        head: Some("abc123".to_string()),
        dirty_files: vec![
            "src/z.rs".to_string(),
            "src/a.rs".to_string(),
            "src/a.rs".to_string(),
        ],
        changed_files: vec!["src/lib.rs".to_string(), "README.md".to_string()],
        diff_stat: Some("2 files changed, 4 insertions(+)".to_string()),
    });
    draft.artifacts = vec![
        RecoveryArtifactInput {
            label: "report".to_string(),
            evidence: evidence("file://workspace/report.md"),
        },
        RecoveryArtifactInput {
            label: "analysis".to_string(),
            evidence: evidence("file://workspace/analysis.md"),
        },
    ];
    draft.last_successful_checkpoint = Some(RecoveryCheckpointInput {
        checkpoint_id: "checkpoint-001".to_string(),
        evidence: evidence("snapshot://snap-000"),
    });
    draft.evidence = vec![
        evidence("file://workspace/notes.md"),
        evidence("snapshot://snap-001"),
    ];

    let capsule = RecoveryCapsule::try_new(draft).expect("recovery capsule");

    assert_eq!(
        capsule
            .completed()
            .iter()
            .map(|item| item.text())
            .collect::<Vec<_>>(),
        vec!["first completed item", "second completed item"]
    );
    let git = capsule.git().expect("git context");
    assert_eq!(git.dirty_files, vec!["src/a.rs", "src/z.rs"]);
    assert_eq!(git.changed_files, vec!["README.md", "src/lib.rs"]);
    assert_eq!(
        capsule
            .artifacts()
            .iter()
            .map(|artifact| artifact.label.as_str())
            .collect::<Vec<_>>(),
        vec!["analysis", "report"]
    );
    assert_eq!(
        capsule
            .evidence()
            .iter()
            .map(EvidenceRef::as_str)
            .collect::<Vec<_>>(),
        vec![
            "event://claude/native-session/completed",
            "event://claude/native-session/goal",
            "file://workspace/analysis.md",
            "file://workspace/notes.md",
            "file://workspace/report.md",
            "snapshot://snap-000",
            "snapshot://snap-001",
        ]
    );
}

#[test]
fn markdown_v1_is_byte_stable_and_keeps_untrusted_text_inside_a_safe_fence() {
    assert_eq!(RECOVERY_CAPSULE_FORMAT, "agentvault.recovery-capsule/v1");
    let mut draft = RecoveryCapsuleDraft::new(identity());
    draft.events = vec![event(
        1,
        RecoveryEventKind::UserMessage,
        "# literal heading\n```unsafe",
        "event://claude/native-session/goal",
    )];
    let capsule = RecoveryCapsule::try_new(draft).expect("recovery capsule");

    let expected = r#"# Recovery Capsule

- Format: `agentvault.recovery-capsule/v1`

## Identity

- Provider: `claude`
- Native session: `native-session`
- Machine: `machine-local`
- Source instance: `claude-home-default`
- Work session: `work-session`
- Project: `agentvault`
- Original cwd: `/workspace/old`
- Current mapped cwd: `/workspace/current`
- Snapshot: `snap-001`
- Captured at: `2026-09-07T08:30:00Z`
- Evidence quality: `verified_snapshot`

## Goal

- Ordinal: `1`
- Provenance: `event://claude/native-session/goal`

````text
# literal heading
```unsafe
````

## Last valid user request

- Ordinal: `1`
- Provenance: `event://claude/native-session/goal`

````text
# literal heading
```unsafe
````

## Recent user context

### Request 1

- Ordinal: `1`
- Provenance: `event://claude/native-session/goal`

````text
# literal heading
```unsafe
````

## Last complete assistant output

_Not available._

## Provider summary

_Not available._

## Current state

_Not available._

## Decisions already made

_Not available._

## Files and Git

_Not available._

## Completed

_Not available._

## Open problems

_Not available._

## Recent failures

_Not available._

## Unfinished TODO

_Not available._

## Artifacts

_Not available._

## Last successful checkpoint

_Not available._

## Recommended next action

_Not available._

## Evidence

- `event://claude/native-session/goal`
- `snapshot://snap-001`
"#;

    assert_eq!(capsule.to_markdown(), expected);
    assert_eq!(capsule.to_markdown(), capsule.to_markdown());
    assert!(!capsule.to_markdown().contains('\r'));
}

#[test]
fn markdown_v1_renders_every_structured_recovery_section() {
    let mut recovery_identity = identity();
    recovery_identity.evidence_quality = EvidenceQuality::TerminalTranscript;
    let mut draft = RecoveryCapsuleDraft::new(recovery_identity);
    draft.events = vec![
        event(
            1,
            RecoveryEventKind::UserMessage,
            "resume the work",
            "event://claude/native-session/user",
        ),
        event(
            2,
            RecoveryEventKind::AssistantOutput { complete: true },
            "implemented the parser",
            "event://claude/native-session/assistant",
        ),
        event(
            3,
            RecoveryEventKind::ProviderSummary,
            "provider summary",
            "event://claude/native-session/summary",
        ),
        event(
            4,
            RecoveryEventKind::ToolFailure,
            "cargo test failed",
            "event://claude/native-session/failure",
        ),
    ];
    draft.current_state = Some("parser complete\nverification pending".to_string());
    draft.decisions = vec![RecoveryNoteInput {
        ordinal: 5,
        text: "keep native sessions read-only".to_string(),
        evidence: Some(evidence("event://claude/native-session/decision")),
    }];
    draft.completed = vec![RecoveryNoteInput {
        ordinal: 6,
        text: "implemented the parser".to_string(),
        evidence: None,
    }];
    draft.open_problems = vec![RecoveryNoteInput {
        ordinal: 7,
        text: "verification is pending".to_string(),
        evidence: None,
    }];
    draft.todos = vec![RecoveryNoteInput {
        ordinal: 8,
        text: "run the full test suite".to_string(),
        evidence: Some(evidence("event://claude/native-session/todo")),
    }];
    draft.recommended_actions = vec![RecoveryNoteInput {
        ordinal: 9,
        text: "verify before continuing".to_string(),
        evidence: None,
    }];
    draft.git = Some(RecoveryGitInput {
        branch: Some("main".to_string()),
        head: Some("abc123".to_string()),
        dirty_files: vec!["src/a.rs".to_string()],
        changed_files: vec!["README.md".to_string(), "src/a.rs".to_string()],
        diff_stat: Some("2 files changed\n4 insertions(+)".to_string()),
    });
    draft.artifacts = vec![RecoveryArtifactInput {
        label: "verification report".to_string(),
        evidence: evidence("file://workspace/report.md"),
    }];
    draft.last_successful_checkpoint = Some(RecoveryCheckpointInput {
        checkpoint_id: "checkpoint-001".to_string(),
        evidence: evidence("snapshot://snap-000"),
    });

    let markdown = RecoveryCapsule::try_new(draft)
        .expect("recovery capsule")
        .to_markdown();

    for expected in [
        "- Evidence quality: `terminal_transcript`",
        "## Current state\n\n```text\nparser complete\nverification pending\n```",
        "## Decisions already made\n\n### Item 1\n\n- Ordinal: `5`\n- Provenance: `event://claude/native-session/decision`",
        "## Files and Git\n\n- Branch: `main`\n- HEAD: `abc123`\n- Dirty: `yes`\n- Dirty files:\n  - `src/a.rs`\n- Changed files:\n  - `README.md`\n  - `src/a.rs`",
        "### Diff stat\n\n```text\n2 files changed\n4 insertions(+)\n```",
        "## Completed\n\n### Item 1\n\n- Ordinal: `6`",
        "## Open problems\n\n### Item 1\n\n- Ordinal: `7`",
        "## Recent failures\n\n### Failure 1\n\n- Ordinal: `4`",
        "## Unfinished TODO\n\n### Item 1\n\n- Ordinal: `8`",
        "## Artifacts\n\n- `verification report`: `file://workspace/report.md`",
        "## Last successful checkpoint\n\n- Checkpoint: `checkpoint-001`\n- Provenance: `snapshot://snap-000`",
        "## Recommended next action\n\n### Item 1\n\n- Ordinal: `9`",
    ] {
        assert!(markdown.contains(expected), "missing markdown fragment: {expected}");
    }
}

#[test]
fn markdown_preserves_captured_at_subsecond_precision() {
    let mut precise_identity = identity();
    precise_identity.captured_at = "2026-09-07T08:30:00.123456Z"
        .parse()
        .expect("precise timestamp");

    let markdown = RecoveryCapsule::try_new(RecoveryCapsuleDraft::new(precise_identity))
        .expect("recovery capsule")
        .to_markdown();

    assert!(markdown.contains("- Captured at: `2026-09-07T08:30:00.123456Z`"));
}

#[test]
fn markdown_inline_code_preserves_values_bounded_by_backticks() {
    let mut quoted_identity = identity();
    quoted_identity.project_id = Some("`quoted-project`".to_string());

    let markdown = RecoveryCapsule::try_new(RecoveryCapsuleDraft::new(quoted_identity))
        .expect("recovery capsule")
        .to_markdown();

    assert!(markdown.contains("- Project: `` `quoted-project` ``"));
}

#[test]
fn evidence_contains_only_events_selected_for_the_capsule() {
    let mut draft = RecoveryCapsuleDraft::new(identity());
    for ordinal in 1..=5 {
        draft.events.push(event(
            ordinal,
            RecoveryEventKind::UserMessage,
            &format!("request {ordinal}"),
            &format!("event://claude/native-session/user-{ordinal}"),
        ));
    }
    for ordinal in 1..=4 {
        draft.events.push(event(
            10 + ordinal,
            RecoveryEventKind::ToolFailure,
            &format!("failure {ordinal}"),
            &format!("event://claude/native-session/failure-{ordinal}"),
        ));
    }
    draft.events.extend([
        event(
            20,
            RecoveryEventKind::ProviderSummary,
            "old summary",
            "event://claude/native-session/summary-old",
        ),
        event(
            21,
            RecoveryEventKind::ProviderSummary,
            "current summary",
            "event://claude/native-session/summary-current",
        ),
        event(
            22,
            RecoveryEventKind::AssistantOutput { complete: false },
            "partial",
            "event://claude/native-session/assistant-partial",
        ),
        event(
            23,
            RecoveryEventKind::AssistantOutput { complete: true },
            "complete",
            "event://claude/native-session/assistant-complete",
        ),
    ]);

    let capsule = RecoveryCapsule::try_new(draft).expect("recovery capsule");
    let evidence = capsule
        .evidence()
        .iter()
        .map(EvidenceRef::as_str)
        .collect::<Vec<_>>();

    for omitted in [
        "event://claude/native-session/user-2",
        "event://claude/native-session/failure-1",
        "event://claude/native-session/summary-old",
        "event://claude/native-session/assistant-partial",
    ] {
        assert!(
            !evidence.contains(&omitted),
            "unexpected evidence: {omitted}"
        );
    }
    for selected in [
        "event://claude/native-session/user-1",
        "event://claude/native-session/user-3",
        "event://claude/native-session/user-4",
        "event://claude/native-session/user-5",
        "event://claude/native-session/failure-2",
        "event://claude/native-session/failure-3",
        "event://claude/native-session/failure-4",
        "event://claude/native-session/summary-current",
        "event://claude/native-session/assistant-complete",
    ] {
        assert!(evidence.contains(&selected), "missing evidence: {selected}");
    }
}
