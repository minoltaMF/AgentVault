//! Read-only provider health diagnostics for AgentVault.
//!
//! The doctor inspects provider metadata, bounded native CLI version output, configured roots,
//! and caller-owned runtime observations. It never discovers transcript contents and it has no
//! repair or persistence API.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use provider_sdk::{
    DetectionContext, ProviderCapabilities, ProviderContext, SessionProvider, SessionRoot,
};

const CLI_OUTPUT_LIMIT: usize = 16 * 1024;
const VERSION_LIMIT: usize = 512;
const MAX_CLI_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderDiagnosticCode {
    ActiveSessionWithoutCheckpoint,
    CleanupDeadlineNear,
    CliMissing,
    CliProbeFailed,
    CliProbeTimedOut,
    DetectionFailed,
    HookInactive,
    NativeIndexInvisible,
    ProjectLocationMoved,
    ProviderNotDetected,
    ReconciliationNotRun,
    RestoreConflict,
    SessionRootsFailed,
    UnknownEvents,
    WatcherInactive,
}

impl ProviderDiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActiveSessionWithoutCheckpoint => "active-session-without-checkpoint",
            Self::CleanupDeadlineNear => "cleanup-deadline-near",
            Self::CliMissing => "cli-missing",
            Self::CliProbeFailed => "cli-probe-failed",
            Self::CliProbeTimedOut => "cli-probe-timed-out",
            Self::DetectionFailed => "detection-failed",
            Self::HookInactive => "hook-inactive",
            Self::NativeIndexInvisible => "native-index-invisible",
            Self::ProjectLocationMoved => "project-location-moved",
            Self::ProviderNotDetected => "provider-not-detected",
            Self::ReconciliationNotRun => "reconciliation-not-run",
            Self::RestoreConflict => "restore-conflict",
            Self::SessionRootsFailed => "session-roots-failed",
            Self::UnknownEvents => "unknown-events",
            Self::WatcherInactive => "watcher-inactive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    pub code: ProviderDiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub summary: String,
    pub recommended_action: String,
}

impl ProviderDiagnostic {
    pub fn new(
        code: ProviderDiagnosticCode,
        severity: DiagnosticSeverity,
        summary: impl Into<String>,
        recommended_action: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity,
            summary: summary.into(),
            recommended_action: recommended_action.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeStatus {
    Active,
    Inactive,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliProbeStatus {
    Available,
    Missing,
    TimedOut,
    Failed,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliObservation {
    pub status: CliProbeStatus,
    pub version: Option<String>,
}

impl CliObservation {
    pub fn available(version: impl Into<String>) -> Self {
        let version = sanitize_version(&version.into());
        Self {
            status: CliProbeStatus::Available,
            version: (!version.is_empty()).then_some(version),
        }
    }

    pub const fn missing() -> Self {
        Self {
            status: CliProbeStatus::Missing,
            version: None,
        }
    }

    pub const fn timed_out() -> Self {
        Self {
            status: CliProbeStatus::TimedOut,
            version: None,
        }
    }

    pub const fn failed() -> Self {
        Self {
            status: CliProbeStatus::Failed,
            version: None,
        }
    }

    pub const fn not_applicable() -> Self {
        Self {
            status: CliProbeStatus::NotApplicable,
            version: None,
        }
    }
}

pub trait CliProbe: Send + Sync {
    fn probe(&self, executable: &str, timeout: Duration) -> CliObservation;
}

/// Runs only `<native-cli> --version`, bounds captured output, and terminates that child on timeout.
#[derive(Debug, Default, Clone, Copy)]
pub struct CommandCliProbe;

impl CliProbe for CommandCliProbe {
    fn probe(&self, executable: &str, timeout: Duration) -> CliObservation {
        run_command_probe(executable, &["--version"], timeout)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DoctorState<'a> {
    pub config_root: &'a Path,
    /// Caller-supplied observation time; the doctor never reads the system clock.
    pub observed_at_ms: i64,
    pub hook_status: RuntimeStatus,
    pub watcher_status: RuntimeStatus,
    pub last_reconciliation_ms: Option<i64>,
    pub parser_version: Option<&'a str>,
    pub unknown_event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDoctorReport {
    pub provider_id: String,
    pub provider_display_name: String,
    pub provider_version: String,
    pub config_root: PathBuf,
    pub observed_at_ms: i64,
    pub cli: CliObservation,
    pub detected: Option<bool>,
    pub detection_evidence: Vec<PathBuf>,
    pub session_roots: Vec<SessionRoot>,
    pub hook_status: RuntimeStatus,
    pub watcher_status: RuntimeStatus,
    pub last_reconciliation_ms: Option<i64>,
    pub parser_version: Option<String>,
    pub unknown_event_count: u64,
    pub resume_capability: bool,
    pub repair_capability: bool,
    pub diagnostics: Vec<ProviderDiagnostic>,
}

pub fn inspect_provider(
    provider: &dyn SessionProvider,
    state: DoctorState<'_>,
    cli_probe: &dyn CliProbe,
) -> ProviderDoctorReport {
    let descriptor = provider.descriptor();
    let cli = descriptor.native_cli.as_deref().map_or_else(
        CliObservation::not_applicable,
        |executable| {
            let timeout = Duration::from_millis(descriptor.health_probe_timeout_ms)
                .clamp(Duration::from_millis(1), MAX_CLI_PROBE_TIMEOUT);
            cli_probe.probe(executable, timeout)
        },
    );
    let mut diagnostics = Vec::new();
    append_cli_diagnostic(&cli, &mut diagnostics);

    let (detected, mut detection_evidence) =
        match provider.detect(&DetectionContext::new(state.config_root)) {
            Ok(result) => {
                if !result.detected {
                    diagnostics.push(ProviderDiagnostic::new(
                        ProviderDiagnosticCode::ProviderNotDetected,
                        DiagnosticSeverity::Warning,
                        "No provider data was detected at the configured root.",
                        "Verify the configured provider root before enabling reconciliation.",
                    ));
                }
                (Some(result.detected), result.evidence)
            }
            Err(_) => {
                diagnostics.push(ProviderDiagnostic::new(
                    ProviderDiagnosticCode::DetectionFailed,
                    DiagnosticSeverity::Error,
                    "The configured provider root could not be inspected safely.",
                    "Review path permissions and reject symlink or reparse-point roots.",
                ));
                (None, Vec::new())
            }
        };
    detection_evidence.sort();
    detection_evidence.dedup();

    let mut session_roots = match provider.roots(&ProviderContext::new(state.config_root)) {
        Ok(roots) => roots,
        Err(_) => {
            diagnostics.push(ProviderDiagnostic::new(
                ProviderDiagnosticCode::SessionRootsFailed,
                DiagnosticSeverity::Error,
                "The provider session roots could not be inspected safely.",
                "Review the configured path without changing native session data.",
            ));
            Vec::new()
        }
    };
    session_roots.sort_by(|left, right| left.path.cmp(&right.path));
    session_roots.dedup_by(|left, right| left == right);

    if state.unknown_event_count > 0 {
        diagnostics.push(ProviderDiagnostic::new(
            ProviderDiagnosticCode::UnknownEvents,
            DiagnosticSeverity::Warning,
            "The parser preserved events it does not understand.",
            "Review parser compatibility before relying on complete projections.",
        ));
    }
    if state.last_reconciliation_ms.is_none() {
        diagnostics.push(ProviderDiagnostic::new(
            ProviderDiagnosticCode::ReconciliationNotRun,
            DiagnosticSeverity::Info,
            "Provider reconciliation has not run yet.",
            "Run a read-only reconciliation before evaluating session coverage.",
        ));
    }
    if descriptor.supports(ProviderCapabilities::LIVE_HOOK)
        && state.hook_status == RuntimeStatus::Inactive
    {
        diagnostics.push(ProviderDiagnostic::new(
            ProviderDiagnosticCode::HookInactive,
            DiagnosticSeverity::Warning,
            "The provider hook is inactive.",
            "Inspect hook configuration; do not edit native sessions to compensate.",
        ));
    }
    if descriptor.supports(ProviderCapabilities::DISCOVER)
        && state.watcher_status == RuntimeStatus::Inactive
    {
        diagnostics.push(ProviderDiagnostic::new(
            ProviderDiagnosticCode::WatcherInactive,
            DiagnosticSeverity::Warning,
            "The provider watcher is inactive.",
            "Run reconciliation or restart the watcher without changing native sessions.",
        ));
    }
    diagnostics.sort_by_key(|diagnostic| (diagnostic.severity, diagnostic.code));
    let resume_capability = descriptor.supports(ProviderCapabilities::NATIVE_RESUME);
    let repair_capability = descriptor.supports(ProviderCapabilities::REPAIR_INDEX);

    ProviderDoctorReport {
        provider_id: descriptor.id,
        provider_display_name: descriptor.display_name,
        provider_version: descriptor.version,
        config_root: state.config_root.to_path_buf(),
        observed_at_ms: state.observed_at_ms,
        cli,
        detected,
        detection_evidence,
        session_roots,
        hook_status: state.hook_status,
        watcher_status: state.watcher_status,
        last_reconciliation_ms: state.last_reconciliation_ms,
        parser_version: state.parser_version.map(str::to_owned),
        unknown_event_count: state.unknown_event_count,
        resume_capability,
        repair_capability,
        diagnostics,
    }
}

fn append_cli_diagnostic(cli: &CliObservation, diagnostics: &mut Vec<ProviderDiagnostic>) {
    let diagnostic = match cli.status {
        CliProbeStatus::Missing => Some(ProviderDiagnostic::new(
            ProviderDiagnosticCode::CliMissing,
            DiagnosticSeverity::Warning,
            "The provider CLI was not found.",
            "Install the native CLI or correct PATH before attempting native resume.",
        )),
        CliProbeStatus::TimedOut => Some(ProviderDiagnostic::new(
            ProviderDiagnosticCode::CliProbeTimedOut,
            DiagnosticSeverity::Error,
            "The provider CLI version probe timed out.",
            "Inspect the native CLI outside AgentVault before enabling process launch.",
        )),
        CliProbeStatus::Failed => Some(ProviderDiagnostic::new(
            ProviderDiagnosticCode::CliProbeFailed,
            DiagnosticSeverity::Warning,
            "The provider CLI version probe failed.",
            "Verify the executable and its version command without changing session data.",
        )),
        CliProbeStatus::Available | CliProbeStatus::NotApplicable => None,
    };
    diagnostics.extend(diagnostic);
}

fn run_command_probe(executable: &str, arguments: &[&str], timeout: Duration) -> CliObservation {
    run_command_probe_with_env(executable, arguments, timeout, &[])
}

fn run_command_probe_with_env(
    executable: &str,
    arguments: &[&str],
    timeout: Duration,
    environment: &[(&str, &str)],
) -> CliObservation {
    if executable.trim().is_empty() || timeout.is_zero() {
        return CliObservation::failed();
    }
    let timeout = timeout.min(MAX_CLI_PROBE_TIMEOUT);
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .envs(environment.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return CliObservation::missing(),
        Err(_) => return CliObservation::failed(),
    };
    let stdout = child.stdout.take().map(spawn_bounded_reader);
    let stderr = child.stderr.take().map(spawn_bounded_reader);
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return CliObservation::timed_out();
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return CliObservation::failed();
            }
        }
    };
    let stdout = receive_output(stdout, deadline);
    let stderr = receive_output(stderr, deadline);
    if !status.success() {
        return CliObservation::failed();
    }
    let output = if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stdout
    } else {
        stderr
    };
    CliObservation::available(String::from_utf8_lossy(&output))
}

fn spawn_bounded_reader<R>(mut reader: R) -> Receiver<Vec<u8>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            let remaining = CLI_OUTPUT_LIMIT.saturating_sub(kept.len());
            kept.extend_from_slice(&chunk[..count.min(remaining)]);
        }
        let _ = sender.send(kept);
    });
    receiver
}

fn receive_output(reader: Option<Receiver<Vec<u8>>>, deadline: Instant) -> Vec<u8> {
    let Some(reader) = reader else {
        return Vec::new();
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        reader.try_recv().unwrap_or_default()
    } else {
        reader
            .recv_timeout(remaining.min(OUTPUT_DRAIN_GRACE))
            .unwrap_or_default()
    }
}

fn sanitize_version(raw: &str) -> String {
    raw.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(VERSION_LIMIT)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_command_is_reported_without_panicking() {
        let observation = run_command_probe(
            "agentvault-command-that-does-not-exist-7f8a",
            &["--version"],
            Duration::from_millis(50),
        );

        assert_eq!(observation.status, CliProbeStatus::Missing);
    }

    #[test]
    fn command_output_is_reduced_to_a_safe_bounded_version_line() {
        assert_eq!(
            sanitize_version("\nagent\u{1b}[31m 1.0\nignored"),
            "agent[31m 1.0"
        );
        assert_eq!(sanitize_version(&"x".repeat(600)).len(), VERSION_LIMIT);
    }

    #[test]
    fn successful_command_probe_captures_a_version_line() {
        let current_exe = std::env::current_exe().expect("current test executable");
        let observation = run_command_probe(
            &current_exe.to_string_lossy(),
            &["--list"],
            Duration::from_secs(1),
        );

        assert_eq!(observation.status, CliProbeStatus::Available);
        assert!(observation.version.is_some());
    }

    #[test]
    fn command_probe_sleep_helper() {
        if std::env::var_os("AGENTVAULT_TEST_CLI_PROBE_SLEEP").is_some() {
            thread::sleep(Duration::from_secs(2));
        }
    }

    #[test]
    fn command_probe_enforces_timeout() {
        let current_exe = std::env::current_exe().expect("current test executable");
        let executable = current_exe.to_string_lossy();
        let observation = run_command_probe_with_env(
            &executable,
            &["--exact", "tests::command_probe_sleep_helper"],
            Duration::from_millis(50),
            &[("AGENTVAULT_TEST_CLI_PROBE_SLEEP", "1")],
        );

        assert_eq!(observation.status, CliProbeStatus::TimedOut);
    }
}
