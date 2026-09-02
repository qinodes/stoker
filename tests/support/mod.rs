#![allow(dead_code)]

use std::path::{Path, PathBuf};

use assert_cmd::Command as AssertCommand;

/// A temporary working directory whose directory is intentionally retained for
/// the lifetime of the test process.
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
            .expect("create temporary working directory")
            .keep();
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
        std::fs::write(path, contents).expect("write working directory file");
    }

    pub fn commit_script(&self, script: &str) -> String {
        self.commit_script_named("script.sh", script)
    }

    pub fn commit_script_named(&self, name: &str, script: &str) -> String {
        self.write(name, format!("{script}\n"));
        String::new()
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

/// Build a Stoker command rooted at `current_dir` with a deterministic
/// temporary home next to that directory.
pub fn stoker_in(current_dir: &Path) -> AssertCommand {
    std::fs::create_dir_all(current_dir).expect("create command working directory");
    let name = current_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("directory");
    let home = current_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}-stoker-home"));
    stoker_with_home_and_dir(&home, current_dir)
}

pub fn stoker_with_home(home: impl AsRef<Path>) -> AssertCommand {
    stoker_with_home_and_dir(home.as_ref(), Path::new("."))
}

pub fn stoker_with_home_and_dir(home: &Path, current_dir: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("stoker").expect("build stoker binary");
    command.current_dir(current_dir).env("STOKER_HOME", home);
    command
}
