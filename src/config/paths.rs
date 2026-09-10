use std::env;
use std::path::PathBuf;

#[cfg(windows)]
use std::ffi::{OsStr, OsString};

const CONFIG_FILE_NAME: &str = "config.json";
const SNAPSHOT_DIR_NAME: &str = "snapshot";

/// Path values for one Stoker installation. Constructing this type performs
/// no filesystem writes; workspace creation is owned by `workspace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StokerPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub runs: PathBuf,
    pub lock: PathBuf,
    pub endpoint: PathBuf,
}

impl StokerPaths {
    pub fn from_env() -> anyhow::Result<Self> {
        let root = match env::var_os("STOKER_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => user_home()?.join(".stoker"),
        };
        Ok(Self {
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
            root,
        })
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE_NAME)
    }

    pub fn snapshot_dir(&self) -> PathBuf {
        self.root.join(SNAPSHOT_DIR_NAME)
    }

    pub fn ipc_endpoint(&self) -> String {
        #[cfg(unix)]
        {
            self.endpoint.to_string_lossy().into_owned()
        }
        #[cfg(windows)]
        {
            format!(r"\\.\pipe\stoker-{:016x}", stable_hash(&self.root))
        }
    }

    pub fn service_log(&self) -> PathBuf {
        self.root.join("service.log")
    }

    pub fn ui_metadata(&self) -> PathBuf {
        self.root.join("ui.json")
    }

    pub fn ui_token(&self) -> PathBuf {
        self.root.join("ui.token")
    }

    pub fn ui_log(&self) -> PathBuf {
        self.root.join("ui.log")
    }
}

/// Keep Windows paths usable as process working directories. Filesystem APIs
/// accept the extended form, while command interpreters do not.
pub(crate) fn normalize_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(windows)]
fn stable_hash(path: &std::path::Path) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn user_home() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    return windows_home_from_vars(
        env::var_os("USERPROFILE").as_deref(),
        env::var_os("HOMEDRIVE").as_deref(),
        env::var_os("HOMEPATH").as_deref(),
        env::var_os("HOME").as_deref(),
    );

    #[cfg(not(windows))]
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("unable to determine the current user's home directory"))
}

#[cfg(windows)]
fn windows_home_from_vars(
    userprofile: Option<&OsStr>,
    homedrive: Option<&OsStr>,
    homepath: Option<&OsStr>,
    home: Option<&OsStr>,
) -> anyhow::Result<PathBuf> {
    if let Some(value) = userprofile.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    if let (Some(drive), Some(path)) = (
        homedrive.filter(|value| !value.is_empty()),
        homepath.filter(|value| !value.is_empty()),
    ) {
        let mut combined = OsString::from(drive);
        combined.push(path);
        return Ok(PathBuf::from(combined));
    }
    if let Some(value) = home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    anyhow::bail!("unable to determine the current user's home directory")
}

#[cfg(all(test, windows))]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::{normalize_path, windows_home_from_vars};

    #[test]
    fn combines_windows_drive_and_home_path() {
        let result = windows_home_from_vars(
            None,
            Some(OsStr::new("C:")),
            Some(OsStr::new(r"\Users\name")),
            None,
        )
        .unwrap();
        assert_eq!(result, Path::new(r"C:\Users\name"));
    }

    #[test]
    fn windows_home_uses_home_fallback_and_reports_missing_home() {
        assert_eq!(
            windows_home_from_vars(None, None, None, Some(OsStr::new(r"D:\Users\fallback")))
                .unwrap(),
            Path::new(r"D:\Users\fallback")
        );
        assert!(windows_home_from_vars(None, None, None, None).is_err());
    }

    #[test]
    fn normalizes_extended_windows_paths_for_command_interpreters() {
        assert_eq!(
            normalize_path(Path::new(r"\\?\UNC\server\share\job").into()),
            Path::new(r"\\server\share\job")
        );
        assert_eq!(
            normalize_path(Path::new(r"\\?\C:\work\job").into()),
            Path::new(r"C:\work\job")
        );
    }
}
