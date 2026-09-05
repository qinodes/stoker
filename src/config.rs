use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::ffi::{OsStr, OsString};

const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StokerConfig {
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimezoneSource {
    Cli,
    Config,
    System,
}

#[derive(Debug, Clone)]
pub struct ResolvedTimezone {
    pub name: String,
    pub source: TimezoneSource,
    timezone: Tz,
}

impl ResolvedTimezone {
    pub fn format(&self, value: DateTime<Utc>) -> String {
        value
            .with_timezone(&self.timezone)
            .to_rfc3339_opts(SecondsFormat::Millis, false)
    }
}

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
        self.initialize_config()?;
        Ok(())
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE_NAME)
    }

    pub fn read_config(&self) -> anyhow::Result<StokerConfig> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(StokerConfig::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("read Stoker config {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("parse Stoker config {}", path.display()))
    }

    pub fn write_config(&self, config: &StokerConfig) -> anyhow::Result<()> {
        let path = self.config_path();
        let contents = serde_json::to_string_pretty(config)? + "\n";
        fs::write(&path, contents)
            .with_context(|| format!("write Stoker config {}", path.display()))?;
        Ok(())
    }

    fn initialize_config(&self) -> anyhow::Result<()> {
        let path = self.config_path();
        if path.exists() {
            return Ok(());
        }

        let Ok(timezone) = system_timezone_name() else {
            return Ok(());
        };
        let contents = serde_json::to_string_pretty(&StokerConfig {
            timezone: Some(timezone),
        })? + "\n";
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("create Stoker config {}", path.display()));
            }
        };
        file.write_all(contents.as_bytes())
            .with_context(|| format!("initialize Stoker config {}", path.display()))?;
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

pub fn system_timezone_name() -> anyhow::Result<String> {
    iana_time_zone::get_timezone().context("detect operating system timezone")
}

pub fn resolve_timezone(
    paths: &StokerPaths,
    cli_timezone: Option<&str>,
) -> anyhow::Result<ResolvedTimezone> {
    let (name, source) = if let Some(value) = cli_timezone {
        (value.to_owned(), TimezoneSource::Cli)
    } else if let Some(value) = paths.read_config()?.timezone {
        (value, TimezoneSource::Config)
    } else {
        (system_timezone_name()?, TimezoneSource::System)
    };
    let timezone = name.parse::<Tz>().map_err(|_| {
        anyhow::anyhow!(
            "unknown timezone {name:?}; use an IANA timezone such as Asia/Taipei or UTC"
        )
    })?;
    Ok(ResolvedTimezone {
        name,
        source,
        timezone,
    })
}

/// Keep Windows paths usable as process working directories. The filesystem
/// APIs accept the extended `\\?\` form, but command interpreters do not.
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

#[cfg(test)]
mod timezone_tests {
    use super::*;
    use chrono::TimeZone;

    fn paths(root: &std::path::Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    #[test]
    fn config_initialization_uses_system_timezone_and_preserves_existing_config() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();

        let initial = paths.read_config().unwrap();
        assert!(initial.timezone.is_some());

        let configured = StokerConfig {
            timezone: Some("Asia/Tokyo".into()),
        };
        paths.write_config(&configured).unwrap();
        paths.ensure().unwrap();
        assert_eq!(paths.read_config().unwrap(), configured);
    }

    #[test]
    fn timezone_resolution_prefers_cli_over_config_and_formats_with_offset() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        paths
            .write_config(&StokerConfig {
                timezone: Some("Asia/Taipei".into()),
            })
            .unwrap();

        let from_config = resolve_timezone(&paths, None).unwrap();
        assert_eq!(from_config.name, "Asia/Taipei");
        assert_eq!(from_config.source, TimezoneSource::Config);

        let from_cli = resolve_timezone(&paths, Some("Asia/Tokyo")).unwrap();
        assert_eq!(from_cli.name, "Asia/Tokyo");
        assert_eq!(from_cli.source, TimezoneSource::Cli);
        let instant = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(from_cli.format(instant), "2026-01-01T09:00:00.000+09:00");
    }
}
