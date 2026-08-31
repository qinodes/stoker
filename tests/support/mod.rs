#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::Command as AssertCommand;

/// A temporary Git repository whose directory is intentionally retained for
/// the lifetime of the test process. This also makes it safe to pass
/// `TestRepo::new().path()` directly to a command builder.
pub struct TestRepo {
    path: PathBuf,
}

/// Isolated Stoker data directory for service/CLI tests that need to share a
/// home across multiple command invocations.
pub struct TempStokerHome {
    path: PathBuf,
}

impl TempStokerHome {
    pub fn new() -> Self {
        Self {
            path: tempfile::tempdir()
                .expect("create temporary Stoker home")
                .keep(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempStokerHome {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl TestRepo {
    pub fn new() -> Self {
        let path = tempfile::tempdir()
            .expect("create temporary repository")
            .keep();
        run_git(&path, ["init"]).expect("initialize temporary repository");
        run_git(
            &path,
            ["config", "user.email", "stoker-tests@example.invalid"],
        )
        .expect("configure Git email");
        run_git(&path, ["config", "user.name", "Stoker Tests"]).expect("configure Git name");
        std::fs::write(path.join("tracked.txt"), "initial\n").expect("write initial file");
        run_git(&path, ["add", "tracked.txt"]).expect("stage initial file");
        run_git(&path, ["commit", "-m", "initial"]).expect("commit initial file");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.path.join(relative)
    }

    pub fn write(&self, relative: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = self.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directories");
        }
        std::fs::write(path, contents).expect("write repository file");
    }

    /// Write a shell script into the repository, commit it, and return the
    /// resulting HEAD. The script itself is useful to later worktree tests;
    /// callers choose the platform command with `TestCommand::shell`.
    pub fn commit_script(&self, script: &str) -> String {
        self.write("script.sh", format!("{script}\n"));
        run_git(&self.path, ["add", "script.sh"]).expect("stage script");
        run_git(&self.path, ["commit", "-m", "script"]).expect("commit script");
        git_output(&self.path, ["rev-parse", "HEAD"])
    }
}

pub struct TestCommand;

impl TestCommand {
    pub fn shell(script: &str) -> Vec<String> {
        #[cfg(unix)]
        {
            vec!["sh".into(), "-c".into(), script.into()]
        }
        #[cfg(windows)]
        {
            vec!["cmd".into(), "/C".into(), script.into()]
        }
    }
}

/// Build a Stoker command rooted at `current_dir` and use a deterministic
/// temporary home outside its Git worktree. Keeping the home outside the
/// repository prevents its SQLite file from making the worktree dirty.
pub fn stoker_in(current_dir: &Path) -> AssertCommand {
    std::fs::create_dir_all(current_dir).expect("create command working directory");
    let root = find_repo_root(current_dir).unwrap_or_else(|| current_dir.to_path_buf());
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    let home = root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}-stoker-home"));
    stoker_with_home_and_dir(&home, current_dir)
}

pub fn stoker_with_home(home: impl AsRef<Path>) -> AssertCommand {
    stoker_with_home_and_dir(home.as_ref(), Path::new("."))
}

fn stoker_with_home_and_dir(home: &Path, current_dir: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("stoker").expect("build stoker binary");
    command.current_dir(current_dir).env("STOKER_HOME", home);
    command
}

fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let mut candidate = path.canonicalize().ok()?;
    if !candidate.is_dir() {
        candidate.pop();
    }
    loop {
        if candidate.join(".git").exists() {
            return Some(candidate);
        }
        if !candidate.pop() {
            return None;
        }
    }
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
