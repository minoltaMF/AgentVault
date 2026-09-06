use std::fmt::Write;

use chrono::SecondsFormat;

use crate::{RecoveryCapsule, RecoveryEventInput, RecoveryGitInput, RecoveryNoteInput};

pub const RECOVERY_CAPSULE_FORMAT: &str = "agentvault.recovery-capsule/v1";

impl RecoveryCapsule {
    /// Render the canonical offline handoff document with LF line endings.
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();
        output.push_str("# Recovery Capsule\n\n- Format: ");
        output.push_str(&inline_code(RECOVERY_CAPSULE_FORMAT));
        output.push_str("\n\n## Identity\n\n");

        let identity = self.identity();
        write_inline_field(&mut output, "Provider", Some(&identity.provider_id));
        write_inline_field(
            &mut output,
            "Native session",
            Some(&identity.native_session_id),
        );
        write_inline_field(&mut output, "Machine", Some(&identity.machine_id));
        write_inline_field(
            &mut output,
            "Source instance",
            Some(&identity.source_instance_id),
        );
        write_inline_field(
            &mut output,
            "Work session",
            identity.work_session_id.as_deref(),
        );
        write_inline_field(&mut output, "Project", identity.project_id.as_deref());
        write_inline_field(
            &mut output,
            "Original cwd",
            identity.original_cwd.as_deref(),
        );
        write_inline_field(
            &mut output,
            "Current mapped cwd",
            identity.current_mapped_cwd.as_deref(),
        );
        write_inline_field(&mut output, "Snapshot", identity.snapshot_id.as_deref());
        write_inline_field(
            &mut output,
            "Captured at",
            Some(
                &identity
                    .captured_at
                    .to_rfc3339_opts(SecondsFormat::AutoSi, true),
            ),
        );
        write_inline_field(
            &mut output,
            "Evidence quality",
            Some(identity.evidence_quality.as_str()),
        );

        write_event_section(&mut output, "Goal", self.goal());
        write_event_section(
            &mut output,
            "Last valid user request",
            self.last_valid_user_request(),
        );
        write_heading(&mut output, "Recent user context");
        if self.recent_user_messages().is_empty() {
            write_unavailable(&mut output);
        } else {
            for (index, event) in self.recent_user_messages().iter().enumerate() {
                let _ = write!(output, "### Request {}\n\n", index + 1);
                write_event(&mut output, event);
            }
        }
        write_event_section(
            &mut output,
            "Last complete assistant output",
            self.last_complete_assistant_output(),
        );
        write_event_section(&mut output, "Provider summary", self.provider_summary());

        write_optional_block_section(&mut output, "Current state", self.current_state());
        write_note_section(&mut output, "Decisions already made", self.decisions());
        write_git_section(&mut output, self.git());
        write_note_section(&mut output, "Completed", self.completed());
        write_note_section(&mut output, "Open problems", self.open_problems());
        write_event_list_section(
            &mut output,
            "Recent failures",
            "Failure",
            self.recent_failures(),
        );
        write_note_section(&mut output, "Unfinished TODO", self.todos());
        write_heading(&mut output, "Artifacts");
        if self.artifacts().is_empty() {
            write_unavailable(&mut output);
        } else {
            for artifact in self.artifacts() {
                output.push_str("- ");
                output.push_str(&inline_code(&artifact.label));
                output.push_str(": ");
                output.push_str(&inline_code(artifact.evidence.as_str()));
                output.push('\n');
            }
        }
        write_heading(&mut output, "Last successful checkpoint");
        if let Some(checkpoint) = self.last_successful_checkpoint() {
            write_inline_field(&mut output, "Checkpoint", Some(&checkpoint.checkpoint_id));
            write_inline_field(
                &mut output,
                "Provenance",
                Some(checkpoint.evidence.as_str()),
            );
        } else {
            write_unavailable(&mut output);
        }
        write_note_section(
            &mut output,
            "Recommended next action",
            self.recommended_actions(),
        );

        write_heading(&mut output, "Evidence");
        if self.evidence().is_empty() {
            write_unavailable(&mut output);
        } else {
            for evidence in self.evidence() {
                output.push_str("- ");
                output.push_str(&inline_code(evidence.as_str()));
                output.push('\n');
            }
        }
        output
    }
}

fn write_heading(output: &mut String, heading: &str) {
    let _ = write!(output, "\n## {heading}\n\n");
}

fn write_inline_field(output: &mut String, label: &str, value: Option<&str>) {
    let _ = write!(output, "- {label}: ");
    match value {
        Some(value) => output.push_str(&inline_code(value)),
        None => output.push_str("_Not available._"),
    }
    output.push('\n');
}

fn write_event_section(output: &mut String, heading: &str, event: Option<&RecoveryEventInput>) {
    write_heading(output, heading);
    match event {
        Some(event) => write_event(output, event),
        None => write_unavailable(output),
    }
}

fn write_event(output: &mut String, event: &RecoveryEventInput) {
    let _ = writeln!(
        output,
        "- Ordinal: {}",
        inline_code(&event.ordinal.to_string())
    );
    output.push_str("- Provenance: ");
    output.push_str(&inline_code(event.evidence.as_str()));
    output.push_str("\n\n");
    write_block(output, event.text());
}

fn write_event_list_section(
    output: &mut String,
    heading: &str,
    item_label: &str,
    events: &[RecoveryEventInput],
) {
    write_heading(output, heading);
    if events.is_empty() {
        write_unavailable(output);
        return;
    }
    for (index, event) in events.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let _ = write!(output, "### {item_label} {}\n\n", index + 1);
        write_event(output, event);
    }
}

fn write_optional_block_section(output: &mut String, heading: &str, value: Option<&str>) {
    write_heading(output, heading);
    match value {
        Some(value) => write_block(output, value),
        None => write_unavailable(output),
    }
}

fn write_note_section(output: &mut String, heading: &str, notes: &[RecoveryNoteInput]) {
    write_heading(output, heading);
    if notes.is_empty() {
        write_unavailable(output);
        return;
    }
    for (index, note) in notes.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let _ = write!(output, "### Item {}\n\n", index + 1);
        let _ = writeln!(
            output,
            "- Ordinal: {}",
            inline_code(&note.ordinal.to_string())
        );
        if let Some(evidence) = &note.evidence {
            write_inline_field(output, "Provenance", Some(evidence.as_str()));
        }
        output.push('\n');
        write_block(output, note.text());
    }
}

fn write_git_section(output: &mut String, git: Option<&RecoveryGitInput>) {
    write_heading(output, "Files and Git");
    let Some(git) = git else {
        write_unavailable(output);
        return;
    };
    write_inline_field(output, "Branch", git.branch.as_deref());
    write_inline_field(output, "HEAD", git.head.as_deref());
    write_inline_field(
        output,
        "Dirty",
        Some(if git.dirty_files.is_empty() {
            "no"
        } else {
            "yes"
        }),
    );
    write_inline_list(output, "Dirty files", &git.dirty_files);
    write_inline_list(output, "Changed files", &git.changed_files);
    if let Some(diff_stat) = &git.diff_stat {
        output.push_str("\n### Diff stat\n\n");
        write_block(output, diff_stat);
    }
}

fn write_inline_list(output: &mut String, label: &str, values: &[String]) {
    let _ = write!(output, "- {label}:");
    if values.is_empty() {
        output.push_str(" _None._\n");
        return;
    }
    output.push('\n');
    for value in values {
        output.push_str("  - ");
        output.push_str(&inline_code(value));
        output.push('\n');
    }
}

fn write_unavailable(output: &mut String) {
    output.push_str("_Not available._\n");
}

fn inline_code(value: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(value) + 1);
    let padding = if value.starts_with('`') || value.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{value}{padding}{fence}")
}

fn write_block(output: &mut String, value: &str) {
    let fence = "`".repeat((longest_backtick_run(value) + 1).max(3));
    let _ = write!(output, "{fence}text\n{value}\n{fence}\n");
}

fn longest_backtick_run(value: &str) -> usize {
    value
        .chars()
        .fold((0, 0), |(longest, current), character| {
            if character == '`' {
                (longest.max(current + 1), current + 1)
            } else {
                (longest, 0)
            }
        })
        .0
}
