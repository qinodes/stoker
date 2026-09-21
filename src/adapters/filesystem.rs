use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::application::model::{LogContent, OutputStream};
use crate::application::ports::{
    FlowAttemptArtifacts, JobArtifacts, JobArtifactsError, WorkingDirectoryResolver,
};
use crate::config::{StokerPaths, normalize_path};
use crate::log_storage;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemWorkingDirectoryResolver;

impl WorkingDirectoryResolver for SystemWorkingDirectoryResolver {
    fn resolve_working_directory(&self, path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err("must be an absolute path".to_owned());
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("cannot be resolved: {error}"))?;
        if !canonical.is_dir() {
            return Err("is not a directory".to_owned());
        }
        Ok(normalize_path(canonical))
    }
}

impl JobArtifacts for StokerPaths {
    fn remove_job_artifacts(&self, id: Uuid) -> Result<(), JobArtifactsError> {
        let run_dir = self.runs.join(id.to_string());
        if run_dir.exists() {
            std::fs::remove_dir_all(&run_dir).map_err(artifact_error)?;
        }
        Ok(())
    }

    fn read_log(
        &self,
        id: Uuid,
        stream: OutputStream,
        max_bytes: Option<usize>,
    ) -> Result<LogContent, JobArtifactsError> {
        let name = match stream {
            OutputStream::Stdout => "stdout.log",
            OutputStream::Stderr => "stderr.log",
        };
        let path = self.runs.join(id.to_string()).join(name);
        read_path_log(&path, max_bytes)
    }
}

impl FlowAttemptArtifacts for StokerPaths {
    fn read_flow_attempt_log(
        &self,
        run_id: Uuid,
        task_id: &str,
        attempt: u32,
        stream: OutputStream,
        max_bytes: Option<usize>,
    ) -> Result<LogContent, JobArtifactsError> {
        let name = match stream {
            OutputStream::Stdout => "stdout.log",
            OutputStream::Stderr => "stderr.log",
        };
        let path = self
            .runs
            .join("flows")
            .join(run_id.to_string())
            .join(task_id)
            .join(format!("attempt-{attempt}"))
            .join(name);
        read_path_log(&path, max_bytes)
    }
}

fn read_path_log(path: &Path, max_bytes: Option<usize>) -> Result<LogContent, JobArtifactsError> {
    let metadata = log_storage::read_metadata(path).map_err(artifact_error)?;
    let segments = log_storage::list_segments(path).map_err(artifact_error)?;
    if segments.is_empty() {
        if let Some(metadata) = metadata {
            return Ok(LogContent {
                bytes: Vec::new(),
                available: metadata.retention_cleaned,
                truncated: metadata.truncated,
                capture_error: metadata.capture_error,
            });
        }
        return Ok(LogContent::default());
    }
    let length = segments
        .iter()
        .map(|segment| std::fs::metadata(segment).map(|metadata| metadata.len()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(artifact_error)?
        .into_iter()
        .sum::<u64>();
    let limit = max_bytes.map(|value| value as u64);
    let truncated = metadata.as_ref().is_some_and(|metadata| metadata.truncated)
        || limit.is_some_and(|limit| length > limit);
    let mut skip = limit.map_or(0, |limit| length.saturating_sub(limit));
    let mut bytes = Vec::new();
    for segment in segments {
        let segment_len = std::fs::metadata(&segment).map_err(artifact_error)?.len();
        if skip >= segment_len {
            skip -= segment_len;
            continue;
        }
        let mut file = File::open(&segment).map_err(artifact_error)?;
        if skip > 0 {
            file.seek(SeekFrom::Start(skip)).map_err(artifact_error)?;
            skip = 0;
        }
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(artifact_error)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
    }
    Ok(LogContent {
        bytes,
        available: true,
        truncated,
        capture_error: metadata.and_then(|metadata| metadata.capture_error),
    })
}

fn artifact_error(error: std::io::Error) -> JobArtifactsError {
    JobArtifactsError::Unavailable {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    #[test]
    fn resolver_and_artifacts_cover_portable_directory_and_tail_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let resolved = SystemWorkingDirectoryResolver
            .resolve_working_directory(directory.path())
            .unwrap();
        assert!(resolved.is_absolute());
        assert!(
            SystemWorkingDirectoryResolver
                .resolve_working_directory(Path::new("relative"))
                .is_err()
        );
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(
            SystemWorkingDirectoryResolver
                .resolve_working_directory(file.path())
                .is_err()
        );

        let id = Uuid::new_v4();
        let paths = paths(directory.path());
        let run = paths.runs.join(id.to_string());
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("stdout.log"), b"0123456789").unwrap();
        let tail = paths.read_log(id, OutputStream::Stdout, Some(4)).unwrap();
        assert_eq!(tail.bytes, b"6789");
        assert!(tail.available);
        assert!(tail.truncated);
        assert_eq!(
            paths.read_log(id, OutputStream::Stderr, Some(4)).unwrap(),
            LogContent::default()
        );
        paths.remove_job_artifacts(id).unwrap();
        assert!(!run.exists());
    }
}
