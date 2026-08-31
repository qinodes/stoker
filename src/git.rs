use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSnapshot {
    pub repository: PathBuf,
    pub git_commit: String,
    /// Alias for callers that use the shorter Git vocabulary.
    pub commit: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("unable to inspect Git repository: {0}")]
    Io(#[from] std::io::Error),
    #[error("not inside a Git repository: {detail}")]
    NotRepository { detail: String },
    #[error("Git repository has uncommitted changes")]
    Dirty,
    #[error("current directory is outside the Git repository")]
    OutsideRepository,
    #[error("Git command `{command}` failed: {detail}")]
    Command { command: String, detail: String },
    #[error("Git command `{command}` returned no output")]
    EmptyOutput { command: String },
}

/// Capture the immutable source snapshot for a submission without invoking a
/// shell. All three Git queries run from `current_dir` and therefore work for
/// both repository roots and nested directories.
pub fn capture_submission(current_dir: impl AsRef<Path>) -> Result<GitSnapshot, GitError> {
    let current_dir = current_dir.as_ref().canonicalize()?;
    if !current_dir.is_dir() {
        return Err(GitError::OutsideRepository);
    }

    let root_output = run_git(&current_dir, &["rev-parse", "--show-toplevel"])?;
    if !root_output.status.success() {
        return Err(GitError::NotRepository {
            detail: command_detail(&root_output),
        });
    }
    let root_text = stdout_text(&root_output);
    if root_text.is_empty() {
        return Err(GitError::EmptyOutput {
            command: "git rev-parse --show-toplevel".into(),
        });
    }
    let repository = PathBuf::from(root_text).canonicalize()?;
    let cwd = current_dir
        .strip_prefix(&repository)
        .map_err(|_| GitError::OutsideRepository)?
        .to_path_buf();

    let status_output = run_git(&current_dir, &["status", "--porcelain"])?;
    if !status_output.status.success() {
        return Err(GitError::Command {
            command: "git status --porcelain".into(),
            detail: command_detail(&status_output),
        });
    }
    if !status_output.stdout.is_empty() {
        return Err(GitError::Dirty);
    }

    let head_output = run_git(&current_dir, &["rev-parse", "HEAD"])?;
    if !head_output.status.success() {
        return Err(GitError::Command {
            command: "git rev-parse HEAD".into(),
            detail: command_detail(&head_output),
        });
    }
    let git_commit = stdout_text(&head_output);
    if git_commit.is_empty() {
        return Err(GitError::EmptyOutput {
            command: "git rev-parse HEAD".into(),
        });
    }

    Ok(GitSnapshot {
        repository,
        git_commit: git_commit.clone(),
        commit: git_commit,
        cwd,
    })
}

pub fn add_detached_worktree(
    repository: &Path,
    target: &Path,
    commit: &str,
) -> Result<(), GitError> {
    run_checked(
        repository,
        &["worktree", "add", "--detach"],
        &[target.to_string_lossy().as_ref(), commit],
        "git worktree add --detach",
    )
}

pub fn remove_worktree(repository: &Path, target: &Path) -> Result<(), GitError> {
    run_checked(
        repository,
        &["worktree", "remove", "--force"],
        &[target.to_string_lossy().as_ref()],
        "git worktree remove --force",
    )
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<Output, GitError> {
    Ok(Command::new("git").args(args).current_dir(cwd).output()?)
}

fn run_checked(
    repository: &Path,
    args: &[&str],
    extra: &[&str],
    command_name: &str,
) -> Result<(), GitError> {
    let mut command = Command::new("git");
    command.current_dir(repository).args(args).args(extra);
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::Command {
            command: command_name.into(),
            detail: command_detail(&output),
        })
    }
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn command_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr
    }
}
