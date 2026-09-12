use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{LogPolicy, StokerPaths};
use crate::domain::{Job, JobState};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LogStreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LogMetadata {
    pub truncated: bool,
    pub dropped_bytes: u64,
    pub earliest_offset: u64,
    pub retained_bytes: u64,
    pub stream: LogStreamKind,
    #[serde(default)]
    pub retention_cleaned: bool,
    #[serde(default)]
    pub capture_error: Option<String>,
}

pub(crate) fn available_space(path: &Path) -> std::io::Result<u64> {
    fs2::available_space(path)
}

pub(crate) fn terminal_log_size(paths: &StokerPaths, id: uuid::Uuid) -> std::io::Result<u64> {
    let directory = paths.runs.join(id.to_string());
    if !directory.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with("stdout") || name.starts_with("stderr"))
            && (name.ends_with(".log") || name.ends_with(".meta.json"))
        {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

/// Remove only terminal-job log artifacts. Database job rows and active run
/// directories are intentionally untouched.
pub(crate) fn enforce_retention(
    paths: &StokerPaths,
    store: &Store,
    policy: LogPolicy,
) -> anyhow::Result<usize> {
    let mut jobs = store
        .list_jobs(None)?
        .into_iter()
        .filter(|job| {
            matches!(
                job.state,
                JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
            )
        })
        .collect::<Vec<_>>();
    jobs.sort_by_key(|job| job.finished_at.or(Some(job.created_at)));
    let mut total = jobs
        .iter()
        .map(|job| terminal_log_size(paths, job.id))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum::<u64>();
    let keep_from = jobs.len().saturating_sub(policy.retention_jobs as usize);
    let mut removed = 0;
    for (index, job) in jobs.iter().enumerate() {
        if index < keep_from || total > policy.max_bytes_total {
            let size = terminal_log_size(paths, job.id)?;
            if size > 0 {
                clean_job_logs(paths, job)?;
                total = total.saturating_sub(size);
                removed += 1;
            }
        }
        if total <= policy.max_bytes_total && index >= keep_from {
            break;
        }
    }
    Ok(removed)
}

fn clean_job_logs(paths: &StokerPaths, job: &Job) -> anyhow::Result<()> {
    let directory = paths.runs.join(job.id.to_string());
    for stream in [LogStreamKind::Stdout, LogStreamKind::Stderr] {
        let name = match stream {
            LogStreamKind::Stdout => "stdout.log",
            LogStreamKind::Stderr => "stderr.log",
        };
        let path = directory.join(name);
        let mut candidates = list_segments(&path)?;
        candidates.push(path.clone());
        candidates.push(path.with_extension("meta.json"));
        for candidate in candidates {
            match fs::remove_file(&candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let metadata = LogMetadata {
            truncated: true,
            dropped_bytes: 0,
            earliest_offset: 0,
            retained_bytes: 0,
            stream,
            retention_cleaned: true,
            capture_error: None,
        };
        futures_write_metadata(&path, &metadata)?;
    }
    Ok(())
}

fn futures_write_metadata(path: &Path, metadata: &LogMetadata) -> anyhow::Result<()> {
    let contents = serde_json::to_vec_pretty(metadata)?;
    fs::write(metadata_path(path), contents)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::{NewJob, Store};

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    fn job(store: &Store, name: &str) -> uuid::Uuid {
        let id = store
            .create_job(NewJob {
                name: name.into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.cancel_not_started(id).unwrap();
        id
    }

    #[test]
    fn retention_removes_old_terminal_logs_but_keeps_job_rows() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        std::fs::create_dir_all(&paths.runs).unwrap();
        let store = Store::open(&paths.database).unwrap();
        let old = job(&store, "old");
        let new = job(&store, "new");
        for (id, bytes) in [(old, b"old".as_slice()), (new, b"new".as_slice())] {
            let run = paths.runs.join(id.to_string());
            std::fs::create_dir_all(&run).unwrap();
            std::fs::write(run.join("stdout.log"), bytes).unwrap();
        }

        let policy = LogPolicy {
            retention_jobs: 1,
            ..LogPolicy::default()
        };
        enforce_retention(&paths, &store, policy).unwrap();

        assert!(!paths.runs.join(old.to_string()).join("stdout.log").exists());
        assert!(
            paths
                .runs
                .join(old.to_string())
                .join("stdout.meta.json")
                .exists()
        );
        assert!(paths.runs.join(new.to_string()).join("stdout.log").exists());
        assert_eq!(store.get_job(old).unwrap().state, JobState::Cancelled);
    }

    #[test]
    fn retention_never_removes_active_job_logs() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        std::fs::create_dir_all(&paths.runs).unwrap();
        let store = Store::open(&paths.database).unwrap();
        let id = store
            .create_job(NewJob {
                name: "active".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        let active = store.claim_next().unwrap().unwrap();
        assert_eq!(active.state, JobState::Starting);
        let run = paths.runs.join(id.to_string());
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("stdout.log"), b"active").unwrap();

        let policy = LogPolicy {
            retention_jobs: 0,
            ..LogPolicy::default()
        };
        enforce_retention(&paths, &store, policy).unwrap();

        assert!(run.join("stdout.log").exists());
    }
}

pub(crate) fn metadata_path(path: &Path) -> PathBuf {
    path.with_extension("meta.json")
}

pub(crate) fn segment_path(path: &Path, index: u64) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("log");
    path.with_file_name(format!("{stem}.{index:06}.log"))
}

/// Return the base log followed by numbered segments, in chronological order.
pub(crate) fn list_segments(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut segments = Vec::new();
    if path.exists() {
        segments.push((0_u64, path.to_path_buf()));
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("log");
    let prefix = format!("{stem}.");
    if let Some(parent) = path.parent()
        && parent.exists()
    {
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(index) = name
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(".log"))
                .filter(|value| value.len() == 6 && value.chars().all(|c| c.is_ascii_digit()))
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            segments.push((index.saturating_add(1), entry.path()));
        }
    }
    segments.sort_by_key(|(order, _)| *order);
    Ok(segments.into_iter().map(|(_, path)| path).collect())
}

pub(crate) async fn write_metadata(path: &Path, metadata: &LogMetadata) -> std::io::Result<()> {
    let contents = serde_json::to_vec_pretty(metadata)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    tokio::fs::write(metadata_path(path), contents).await
}

pub(crate) fn read_metadata(path: &Path) -> std::io::Result<Option<LogMetadata>> {
    let path = metadata_path(path);
    match std::fs::read(path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map(Some)
            .map_err(|error| std::io::Error::other(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
