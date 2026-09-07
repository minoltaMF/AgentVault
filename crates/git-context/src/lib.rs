//! Read-only Git checkpoint capture for AgentVault.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub type GitCheckpointResult<T> = Result<T, GitCheckpointError>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GitDiffStat {
    insertions: u64,
    deletions: u64,
    binary_files: u64,
}

impl GitDiffStat {
    pub const fn insertions(self) -> u64 {
        self.insertions
    }

    pub const fn deletions(self) -> u64 {
        self.deletions
    }

    pub const fn binary_files(self) -> u64 {
        self.binary_files
    }
}

/// A point-in-time, metadata-only description of one Git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCheckpoint {
    repository_root: PathBuf,
    branch: Option<String>,
    head_commit: Option<String>,
    changed_files: Vec<String>,
    untracked_files: Vec<String>,
    diff_stat: GitDiffStat,
}

impl GitCheckpoint {
    pub fn repository_root(&self) -> &Path {
        self.repository_root.as_path()
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn head_commit(&self) -> Option<&str> {
        self.head_commit.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        !self.changed_files.is_empty() || !self.untracked_files.is_empty()
    }

    pub fn changed_files(&self) -> &[String] {
        &self.changed_files
    }

    pub fn untracked_files(&self) -> &[String] {
        &self.untracked_files
    }

    pub const fn diff_stat(&self) -> GitDiffStat {
        self.diff_stat
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitCheckpointError {
    #[error("Git {operation} could not be started: {source}")]
    GitUnavailable {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("path is not inside a Git working tree: {0}")]
    NotRepository(PathBuf),
    #[error("Git {operation} failed: {message}")]
    CommandFailed {
        operation: &'static str,
        message: String,
    },
    #[error("Git {operation} output is not valid UTF-8")]
    NonUtf8Output { operation: &'static str },
    #[error("Git {operation} output is malformed")]
    MalformedOutput { operation: &'static str },
    #[error("repository root cannot be resolved: {0}")]
    RepositoryRoot(#[source] std::io::Error),
}

/// Capture Git metadata without modifying the worktree, index, refs, or object database.
pub fn capture_git_checkpoint(path: impl AsRef<Path>) -> GitCheckpointResult<GitCheckpoint> {
    let requested_path = path.as_ref();
    let root_output = git_output(
        requested_path,
        "repository discovery",
        &["rev-parse", "--show-toplevel"],
    )?;
    if !root_output.status.success() {
        return Err(GitCheckpointError::NotRepository(
            requested_path.to_path_buf(),
        ));
    }
    let repository_root = fs::canonicalize(parse_single_line(
        "repository discovery",
        &root_output.stdout,
    )?)
    .map_err(GitCheckpointError::RepositoryRoot)?;

    let branch = optional_single_line(
        &repository_root,
        "branch discovery",
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    let head_commit = optional_single_line(
        &repository_root,
        "HEAD discovery",
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )?;

    let status = successful_git_output(
        &repository_root,
        "status",
        &[
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ],
    )?;
    let (mut changed_files, mut untracked_files) = parse_status(&status.stdout)?;
    changed_files.sort();
    changed_files.dedup();
    untracked_files.sort();
    untracked_files.dedup();

    let diff_args = if head_commit.is_some() {
        [
            "diff",
            "--numstat",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ]
        .as_slice()
    } else {
        [
            "diff",
            "--numstat",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--cached",
            "--",
        ]
        .as_slice()
    };
    let diff_output = successful_git_output(&repository_root, "diff stat", diff_args)?;
    let diff_stat = parse_numstat(&diff_output.stdout)?;

    Ok(GitCheckpoint {
        repository_root,
        branch,
        head_commit,
        changed_files,
        untracked_files,
        diff_stat,
    })
}

fn git_output(path: &Path, operation: &'static str, args: &[&str]) -> GitCheckpointResult<Output> {
    Command::new("git")
        .arg("--no-optional-locks")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.untrackedCache=false")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_DIFF_OPTS")
        .env_remove("GIT_CONFIG_COUNT")
        .output()
        .map_err(|source| GitCheckpointError::GitUnavailable { operation, source })
}

fn successful_git_output(
    path: &Path,
    operation: &'static str,
    args: &[&str],
) -> GitCheckpointResult<Output> {
    let output = git_output(path, operation, args)?;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_failed(operation, &output.stderr))
}

fn optional_single_line(
    path: &Path,
    operation: &'static str,
    args: &[&str],
) -> GitCheckpointResult<Option<String>> {
    let output = git_output(path, operation, args)?;
    if output.status.success() {
        return parse_single_line(operation, &output.stdout).map(Some);
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(command_failed(operation, &output.stderr))
}

fn parse_single_line(operation: &'static str, bytes: &[u8]) -> GitCheckpointResult<String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| GitCheckpointError::NonUtf8Output { operation })?
        .trim();
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err(GitCheckpointError::MalformedOutput { operation });
    }
    Ok(value.to_string())
}

fn parse_status(bytes: &[u8]) -> GitCheckpointResult<(Vec<String>, Vec<String>)> {
    let mut changed = Vec::new();
    let mut untracked = Vec::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if record.len() < 4 || record[2] != b' ' {
            return Err(GitCheckpointError::MalformedOutput {
                operation: "status",
            });
        }
        let path = std::str::from_utf8(&record[3..])
            .map_err(|_| GitCheckpointError::NonUtf8Output {
                operation: "status",
            })?
            .to_string();
        match &record[..2] {
            b"??" => untracked.push(path),
            b"!!" => {}
            _ => changed.push(path),
        }
    }
    Ok((changed, untracked))
}

fn parse_numstat(bytes: &[u8]) -> GitCheckpointResult<GitDiffStat> {
    let output = std::str::from_utf8(bytes).map_err(|_| GitCheckpointError::NonUtf8Output {
        operation: "diff stat",
    })?;
    let mut stat = GitDiffStat::default();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.splitn(3, '\t');
        let insertions = fields.next().ok_or(GitCheckpointError::MalformedOutput {
            operation: "diff stat",
        })?;
        let deletions = fields.next().ok_or(GitCheckpointError::MalformedOutput {
            operation: "diff stat",
        })?;
        fields.next().ok_or(GitCheckpointError::MalformedOutput {
            operation: "diff stat",
        })?;
        if insertions == "-" && deletions == "-" {
            stat.binary_files += 1;
            continue;
        }
        stat.insertions +=
            insertions
                .parse::<u64>()
                .map_err(|_| GitCheckpointError::MalformedOutput {
                    operation: "diff stat",
                })?;
        stat.deletions +=
            deletions
                .parse::<u64>()
                .map_err(|_| GitCheckpointError::MalformedOutput {
                    operation: "diff stat",
                })?;
    }
    Ok(stat)
}

fn command_failed(operation: &'static str, stderr: &[u8]) -> GitCheckpointError {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    GitCheckpointError::CommandFailed { operation, message }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::{capture_git_checkpoint, GitCheckpointError};

    fn git(repository: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .output()
            .expect("git must be available for contract tests");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("fixture output is UTF-8")
            .trim()
            .to_string()
    }

    fn initialized_repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temp repository");
        git(directory.path(), &["init", "--initial-branch=main"]);
        git(
            directory.path(),
            &["config", "user.name", "AgentVault Test"],
        );
        git(
            directory.path(),
            &["config", "user.email", "agentvault@example.invalid"],
        );
        fs::write(directory.path().join("tracked.txt"), "first line\n").expect("write fixture");
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-m", "initial fixture"]);
        directory
    }

    fn unborn_repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temp repository");
        git(directory.path(), &["init", "--initial-branch=main"]);
        directory
    }

    #[test]
    fn captures_clean_repository_identity_and_zero_diff_stat() {
        let repository = initialized_repository();
        let expected_head = git(repository.path(), &["rev-parse", "HEAD"]);

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(
            checkpoint.repository_root(),
            fs::canonicalize(repository.path()).expect("canonical fixture path")
        );
        assert_eq!(checkpoint.branch(), Some("main"));
        assert_eq!(checkpoint.head_commit(), Some(expected_head.as_str()));
        assert!(!checkpoint.is_dirty());
        assert!(checkpoint.changed_files().is_empty());
        assert!(checkpoint.untracked_files().is_empty());
        assert_eq!(checkpoint.diff_stat().insertions(), 0);
        assert_eq!(checkpoint.diff_stat().deletions(), 0);
        assert_eq!(checkpoint.diff_stat().binary_files(), 0);
    }

    #[test]
    fn captures_staged_changes_before_the_first_commit() {
        let repository = unborn_repository();
        fs::write(repository.path().join("draft.txt"), "first\nsecond\n").expect("write fixture");
        git(repository.path(), &["add", "draft.txt"]);

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(checkpoint.branch(), Some("main"));
        assert_eq!(checkpoint.head_commit(), None);
        assert!(checkpoint.is_dirty());
        assert_eq!(checkpoint.changed_files(), &["draft.txt"]);
        assert!(checkpoint.untracked_files().is_empty());
        assert_eq!(checkpoint.diff_stat().insertions(), 2);
        assert_eq!(checkpoint.diff_stat().deletions(), 0);
    }

    #[test]
    fn captures_staged_unstaged_binary_and_untracked_changes_without_contents() {
        let repository = initialized_repository();
        fs::write(repository.path().join("binary.bin"), [0_u8, 1, 2])
            .expect("write binary fixture");
        git(repository.path(), &["add", "binary.bin"]);
        git(repository.path(), &["commit", "-m", "add binary fixture"]);

        fs::write(
            repository.path().join("tracked.txt"),
            "first line\nstaged line\n",
        )
        .expect("write staged fixture");
        git(repository.path(), &["add", "tracked.txt"]);
        fs::write(
            repository.path().join("tracked.txt"),
            "first line\nstaged line\nworking line\n",
        )
        .expect("write unstaged fixture");
        fs::write(repository.path().join("binary.bin"), [0_u8, 3, 4])
            .expect("modify binary fixture");
        fs::create_dir(repository.path().join("notes")).expect("create notes directory");
        fs::write(
            repository.path().join("notes").join("恢复 plan.md"),
            "not copied into the checkpoint\n",
        )
        .expect("write untracked fixture");

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert!(checkpoint.is_dirty());
        assert_eq!(checkpoint.changed_files(), &["binary.bin", "tracked.txt"]);
        assert_eq!(checkpoint.untracked_files(), &["notes/恢复 plan.md"]);
        assert_eq!(checkpoint.diff_stat().insertions(), 2);
        assert_eq!(checkpoint.diff_stat().deletions(), 0);
        assert_eq!(checkpoint.diff_stat().binary_files(), 1);
    }

    #[test]
    fn detached_head_is_recorded_without_inventing_a_branch() {
        let repository = initialized_repository();
        let expected_head = git(repository.path(), &["rev-parse", "HEAD"]);
        git(repository.path(), &["checkout", "--detach", "HEAD"]);

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(checkpoint.branch(), None);
        assert_eq!(checkpoint.head_commit(), Some(expected_head.as_str()));
        assert!(!checkpoint.is_dirty());
    }

    #[test]
    fn rejects_paths_outside_a_git_working_tree() {
        let directory = tempfile::tempdir().expect("temp directory");

        let error = capture_git_checkpoint(directory.path()).expect_err("reject non-repository");

        assert!(
            matches!(error, GitCheckpointError::NotRepository(path) if path == directory.path())
        );
    }

    #[test]
    fn capture_does_not_run_configured_external_diff_commands() {
        let repository = initialized_repository();
        git(
            repository.path(),
            &[
                "config",
                "diff.external",
                "agentvault-external-diff-must-not-run",
            ],
        );
        fs::write(repository.path().join("tracked.txt"), "changed\n").expect("modify fixture");

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(checkpoint.changed_files(), &["tracked.txt"]);
        assert_eq!(checkpoint.diff_stat().insertions(), 1);
        assert_eq!(checkpoint.diff_stat().deletions(), 1);
    }

    #[test]
    fn capture_does_not_run_configured_textconv_commands() {
        let repository = initialized_repository();
        fs::write(
            repository.path().join(".gitattributes"),
            "tracked.txt diff=agentvault-test\n",
        )
        .expect("write attributes");
        git(repository.path(), &["add", ".gitattributes"]);
        git(repository.path(), &["commit", "-m", "add diff attributes"]);
        git(
            repository.path(),
            &[
                "config",
                "diff.agentvault-test.textconv",
                "agentvault-textconv-must-not-run",
            ],
        );
        fs::write(repository.path().join("tracked.txt"), "changed\n").expect("modify fixture");

        let checkpoint = capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(checkpoint.changed_files(), &["tracked.txt"]);
        assert_eq!(checkpoint.diff_stat().insertions(), 1);
        assert_eq!(checkpoint.diff_stat().deletions(), 1);
    }

    #[test]
    fn capture_does_not_run_a_repository_fsmonitor_command() {
        let repository = initialized_repository();
        let marker = repository.path().join("fsmonitor-invoked");
        #[cfg(windows)]
        let monitor = {
            let path = repository
                .path()
                .join(".git")
                .join("agentvault-fsmonitor.cmd");
            fs::write(
                &path,
                "@echo off\r\necho invoked>fsmonitor-invoked\r\nexit /b 1\r\n",
            )
            .expect("write fsmonitor fixture");
            path
        };
        #[cfg(unix)]
        let monitor = {
            use std::os::unix::fs::PermissionsExt;

            let path = repository.path().join(".git").join("agentvault-fsmonitor");
            fs::write(
                &path,
                "#!/bin/sh\nprintf invoked > fsmonitor-invoked\nexit 1\n",
            )
            .expect("write fsmonitor fixture");
            let mut permissions = fs::metadata(&path)
                .expect("fsmonitor metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions).expect("make fsmonitor executable");
            path
        };
        git(
            repository.path(),
            &[
                "config",
                "core.fsmonitor",
                &monitor.to_string_lossy().replace('\\', "/"),
            ],
        );

        capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert!(
            !marker.exists(),
            "configured fsmonitor command was executed"
        );
    }

    #[test]
    fn capture_leaves_index_refs_and_objects_unchanged() {
        let repository = initialized_repository();
        fs::write(repository.path().join("tracked.txt"), "changed\n").expect("modify fixture");
        let git_directory = repository.path().join(".git");
        let index_before = fs::read(git_directory.join("index")).expect("read index before");
        let head_before = fs::read(git_directory.join("HEAD")).expect("read HEAD before");
        let objects_before = git(repository.path(), &["count-objects", "-v"]);

        capture_git_checkpoint(repository.path()).expect("capture checkpoint");

        assert_eq!(
            fs::read(git_directory.join("index")).expect("read index after"),
            index_before
        );
        assert_eq!(
            fs::read(git_directory.join("HEAD")).expect("read HEAD after"),
            head_before
        );
        assert_eq!(
            git(repository.path(), &["count-objects", "-v"]),
            objects_before
        );
    }
}
