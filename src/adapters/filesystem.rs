use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::application::model::{LogContent, OutputStream};
use crate::application::ports::{JobArtifacts, JobArtifactsError, WorkingDirectoryResolver};
use crate::config::{StokerPaths, normalize_path};

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
        if !path.exists() {
            return Ok(LogContent::default());
        }
        let mut file = File::open(&path).map_err(artifact_error)?;
        let length = file.metadata().map_err(artifact_error)?.len();
        let limit = max_bytes.map(|value| value as u64);
        let truncated = limit.is_some_and(|limit| length > limit);
        if let Some(limit) = limit
            && length > limit
        {
            file.seek(SeekFrom::Start(length - limit))
                .map_err(artifact_error)?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(artifact_error)?;
        Ok(LogContent {
            bytes,
            available: true,
            truncated,
        })
    }
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
