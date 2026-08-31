use std::env;
use std::path::PathBuf;

#[cfg(windows)]
use std::ffi::{OsStr, OsString};

/// Paths used by one Stoker installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StokerPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub runs: PathBuf,
    pub lock: PathBuf,
    pub endpoint: PathBuf,
}

impl StokerPaths {
    /// Build paths from STOKER_HOME, or from the current user's home directory.
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

    /// Create the installation directories needed before opening the store.
    pub fn ensure(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(&self.runs)?;
        Ok(())
    }

    /// The operating-system endpoint used for local service IPC.
    ///
    /// Unix uses the filesystem socket in `endpoint`; Windows named pipes use
    /// a stable name derived from the Stoker home directory instead.
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

    pub fn service_log(&self) -> std::path::PathBuf {
        self.root.join("service.log")
    }
}

#[cfg(windows)]
fn stable_hash(path: &std::path::Path) -> u64 {
    // FNV-1a is intentionally used instead of DefaultHasher, whose random
    // per-process seed would produce a different pipe name after each start.
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
        // HOMEDRIVE and HOMEPATH are separate Windows environment variables;
        // joining PathBufs is incorrect for a root-relative HOMEPATH, so join
        // their raw values into the canonical `C:\Users\name` form.
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
    use super::windows_home_from_vars;
    use std::ffi::OsStr;
    use std::path::Path;

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
}
