use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::task::JoinHandle;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_detached;

/// The immutable inputs needed to start one managed process.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessLaunchPolicy {
    pub log_policy: LogCapturePolicy,
    pub termination_grace: std::time::Duration,
}

/// Limits applied independently by the two pipe writers while sharing the
/// per-job byte budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogCapturePolicy {
    pub max_bytes_per_job: u64,
    pub segment_bytes: u64,
}

impl Default for LogCapturePolicy {
    fn default() -> Self {
        Self {
            max_bytes_per_job: crate::config::DEFAULT_LOG_MAX_BYTES_PER_JOB,
            segment_bytes: crate::config::DEFAULT_LOG_SEGMENT_BYTES,
        }
    }
}

/// A process whose output and descendants are managed until completion.
#[async_trait]
pub trait ManagedProcess: Send {
    fn pid(&self) -> u32;
    async fn wait(self: Box<Self>) -> io::Result<ExitStatus>;
    async fn wait_with_cancel(
        self: Box<Self>,
        _cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> io::Result<ExitStatus> {
        self.wait().await
    }
    async fn terminate_tree(&mut self) -> io::Result<()>;
}

/// Starts managed processes using the host platform's process-tree primitive.
#[async_trait]
pub trait ProcessController: Send + Sync {
    async fn spawn(&self, spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>>;

    async fn spawn_with_policy(
        &self,
        spec: ProcessSpec,
        _policy: ProcessLaunchPolicy,
    ) -> io::Result<Box<dyn ManagedProcess>> {
        self.spawn(spec).await
    }
}

/// The default process controller for the current platform.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultProcessController;

impl DefaultProcessController {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessController for DefaultProcessController {
    async fn spawn(&self, spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
        #[cfg(unix)]
        {
            unix::spawn(
                spec,
                ProcessLaunchPolicy {
                    log_policy: Default::default(),
                    termination_grace: std::time::Duration::from_millis(
                        crate::config::DEFAULT_TERMINATION_GRACE_MS,
                    ),
                },
            )
            .await
        }
        #[cfg(windows)]
        {
            windows::spawn(
                spec,
                ProcessLaunchPolicy {
                    log_policy: Default::default(),
                    termination_grace: std::time::Duration::from_millis(
                        crate::config::DEFAULT_TERMINATION_GRACE_MS,
                    ),
                },
            )
            .await
        }
    }

    async fn spawn_with_policy(
        &self,
        spec: ProcessSpec,
        policy: ProcessLaunchPolicy,
    ) -> io::Result<Box<dyn ManagedProcess>> {
        #[cfg(unix)]
        {
            unix::spawn(spec, policy).await
        }
        #[cfg(windows)]
        {
            windows::spawn(spec, policy).await
        }
    }
}

/// A descriptive alias for callers that prefer to name the concrete system
/// implementation rather than the controller trait.
pub type SystemProcessController = DefaultProcessController;

/// Configure a long-running helper so it is independent from the terminal
/// which launched `stoker start` or `stoker ui start`.
pub(crate) fn configure_detached(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // A new process group prevents console control events from being
        // inherited, while DETACHED_PROCESS removes the console association.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // SAFETY: setsid is an async-signal-safe syscall and no allocation or
        // shared-state access occurs in the post-fork child hook.
        unsafe {
            command.pre_exec(|| {
                nix::unistd::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
    }
}

/// Spawn a configured long-running helper without leaking the launcher's
/// redirected standard-stream handles into the detached child.
pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    configure_detached(command);

    #[cfg(windows)]
    {
        windows_detached::spawn(command)
    }

    #[cfg(not(windows))]
    {
        command.spawn()
    }
}

pub(crate) fn spawn_pipe_writer<R>(
    reader: R,
    path: PathBuf,
    policy: LogCapturePolicy,
) -> JoinHandle<io::Result<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let mut writer = match BoundedLogWriter::new(path, policy).await {
            Ok(writer) => Some(writer),
            Err(error) => {
                // The child must still be drained even when the destination
                // cannot be opened; otherwise a full pipe can deadlock wait().
                let open_error = error;
                let mut buffer = [0_u8; 16 * 1024];
                while reader.read(&mut buffer).await? != 0 {}
                return Err(open_error);
            }
        };
        let mut write_error = None;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let bytes_read = reader.read(&mut buffer).await?;
            if bytes_read == 0 {
                if let Some(writer) = writer.as_mut()
                    && let Err(error) = writer.finish().await
                {
                    write_error.get_or_insert(error);
                }
                return write_error.map_or(Ok(()), Err);
            }
            if let Some(output) = writer.as_mut()
                && let Err(error) = output.write(&buffer[..bytes_read]).await
            {
                let metadata = crate::log_storage::LogMetadata {
                    truncated: true,
                    dropped_bytes: output.dropped_bytes,
                    earliest_offset: output.dropped_bytes,
                    retained_bytes: output.retained_bytes,
                    stream: output.stream_kind(),
                    retention_cleaned: false,
                    capture_error: Some(error.to_string()),
                };
                let _ = crate::log_storage::write_metadata(&output.base_path, &metadata).await;
                write_error.get_or_insert(error);
                writer = None;
            }
        }
    })
}

struct BoundedLogWriter {
    base_path: PathBuf,
    current_path: PathBuf,
    file: tokio::fs::File,
    limit: u64,
    segment_bytes: u64,
    current_length: u64,
    retained_bytes: u64,
    closed_segments: VecDeque<(PathBuf, u64)>,
    next_segment: u64,
    dropped_bytes: u64,
}

impl BoundedLogWriter {
    async fn new(path: PathBuf, policy: LogCapturePolicy) -> io::Result<Self> {
        let limit = policy.max_bytes_per_job / 2;
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "per-job log limit must allow both stdout and stderr",
            ));
        }
        let segments = crate::log_storage::list_segments(&path)?;
        let current_path = segments.last().cloned().unwrap_or_else(|| path.clone());
        let current_length = if current_path.exists() {
            std::fs::metadata(&current_path)?.len()
        } else {
            0
        };
        let retained_bytes = segments
            .iter()
            .map(|segment| std::fs::metadata(segment).map(|metadata| metadata.len()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&current_path)
            .await?;
        let mut closed_segments = VecDeque::new();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            closed_segments.push_back((segment.clone(), std::fs::metadata(segment)?.len()));
        }
        Ok(Self {
            base_path: path,
            current_path,
            file,
            limit,
            segment_bytes: policy.segment_bytes.max(1),
            current_length,
            retained_bytes,
            closed_segments,
            next_segment: segments.len() as u64,
            dropped_bytes: 0,
        })
    }

    async fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            if self.current_length >= self.segment_bytes {
                self.rotate().await?;
            }
            if self.retained_bytes >= self.limit && !self.closed_segments.is_empty() {
                self.discard_oldest().await?;
            }
            let available_quota = self.limit.saturating_sub(self.retained_bytes);
            if available_quota == 0 {
                self.dropped_bytes = self
                    .dropped_bytes
                    .saturating_add((bytes.len() - written) as u64);
                return Ok(());
            }
            let available_segment = self.segment_bytes - self.current_length;
            let take = available_quota
                .min(available_segment)
                .min((bytes.len() - written) as u64) as usize;
            self.file.seek(SeekFrom::End(0)).await?;
            self.file.write_all(&bytes[written..written + take]).await?;
            self.current_length += take as u64;
            self.retained_bytes += take as u64;
            written += take;
        }
        Ok(())
    }

    async fn rotate(&mut self) -> io::Result<()> {
        self.file.flush().await?;
        let next = loop {
            let candidate = crate::log_storage::segment_path(&self.base_path, self.next_segment);
            self.next_segment = self.next_segment.saturating_add(1);
            if !candidate.exists() {
                break candidate;
            }
        };
        let new_file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&next)
            .await?;
        let old_file = std::mem::replace(&mut self.file, new_file);
        drop(old_file);
        self.closed_segments
            .push_back((self.current_path.clone(), self.current_length));
        self.current_path = next;
        self.current_length = 0;
        Ok(())
    }

    async fn discard_oldest(&mut self) -> io::Result<()> {
        let Some((oldest, length)) = self.closed_segments.pop_front() else {
            return Ok(());
        };
        let result = match tokio::fs::remove_file(oldest).await {
            Ok(()) => {
                self.retained_bytes = self.retained_bytes.saturating_sub(length);
                self.dropped_bytes = self.dropped_bytes.saturating_add(length);
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.retained_bytes = self.retained_bytes.saturating_sub(length);
                self.dropped_bytes = self.dropped_bytes.saturating_add(length);
                Ok(())
            }
            Err(error) => Err(error),
        };
        if result.is_ok() {
            let _ = crate::log_storage::write_metadata(
                &self.base_path,
                &crate::log_storage::LogMetadata {
                    truncated: self.dropped_bytes > 0,
                    dropped_bytes: self.dropped_bytes,
                    earliest_offset: self.dropped_bytes,
                    retained_bytes: self.retained_bytes,
                    stream: self.stream_kind(),
                    retention_cleaned: false,
                    capture_error: None,
                },
            )
            .await;
        }
        result
    }

    async fn finish(&mut self) -> io::Result<()> {
        self.file.flush().await?;
        let stream = self.stream_kind();
        crate::log_storage::write_metadata(
            &self.base_path,
            &crate::log_storage::LogMetadata {
                truncated: self.dropped_bytes > 0,
                dropped_bytes: self.dropped_bytes,
                earliest_offset: self.dropped_bytes,
                retained_bytes: self.retained_bytes,
                stream,
                retention_cleaned: false,
                capture_error: None,
            },
        )
        .await
    }

    fn stream_kind(&self) -> crate::log_storage::LogStreamKind {
        if self
            .base_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("stderr"))
        {
            crate::log_storage::LogStreamKind::Stderr
        } else {
            crate::log_storage::LogStreamKind::Stdout
        }
    }
}

pub(crate) async fn finish_pipe_writer(task: JoinHandle<io::Result<()>>) -> io::Result<()> {
    match task.await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(format!(
            "process output task failed: {error}"
        ))),
    }
}

pub(crate) async fn finish_pipes(
    stdout_task: JoinHandle<io::Result<()>>,
    stderr_task: JoinHandle<io::Result<()>>,
) -> io::Result<()> {
    let stdout_result = finish_pipe_writer(stdout_task).await;
    let stderr_result = finish_pipe_writer(stderr_task).await;
    stdout_result.and(stderr_result)
}

#[cfg(test)]
mod tests {
    use super::{
        DefaultProcessController, LogCapturePolicy, ManagedProcess, finish_pipe_writer,
        finish_pipes, spawn_pipe_writer,
    };
    use async_trait::async_trait;
    use std::process::ExitStatus;
    use tokio::io::AsyncWriteExt;

    struct DefaultWaitProcess;

    #[async_trait]
    impl ManagedProcess for DefaultWaitProcess {
        fn pid(&self) -> u32 {
            1
        }

        async fn wait(self: Box<Self>) -> std::io::Result<ExitStatus> {
            #[cfg(unix)]
            use std::os::unix::process::ExitStatusExt;
            #[cfg(windows)]
            use std::os::windows::process::ExitStatusExt;

            Ok(ExitStatus::from_raw(0))
        }

        async fn terminate_tree(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn default_process_controller_can_be_constructed() {
        let _ = DefaultProcessController::new();
    }

    #[tokio::test]
    async fn managed_process_default_wait_with_cancel_delegates_to_wait() {
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let mut process = Box::new(DefaultWaitProcess);
        assert_eq!(process.pid(), 1);
        process.terminate_tree().await.unwrap();
        let status = process.wait_with_cancel(cancel_rx).await.unwrap();
        assert_eq!(status.code(), Some(0));
    }

    #[tokio::test]
    async fn pipe_writer_appends_output_and_flushes_before_finishing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout.log");
        let (mut writer, reader) = tokio::io::duplex(128);
        let task = spawn_pipe_writer(reader, path.clone(), Default::default());

        writer.write_all(b"first\n").await.unwrap();
        writer.write_all(b"second\n").await.unwrap();
        writer.shutdown().await.unwrap();

        finish_pipe_writer(task).await.unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "first\nsecond\n");
    }

    #[tokio::test]
    async fn pipe_writer_keeps_only_the_newest_bytes_after_reaching_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout.log");
        let (mut writer, reader) = tokio::io::duplex(128);
        let task = spawn_pipe_writer(
            reader,
            path.clone(),
            LogCapturePolicy {
                max_bytes_per_job: 32,
                segment_bytes: 8,
            },
        );

        writer
            .write_all(b"abcdefghijklmnopqrstuvwxyz")
            .await
            .unwrap();
        writer.shutdown().await.unwrap();
        finish_pipe_writer(task).await.unwrap();

        let segments = crate::log_storage::list_segments(&path).unwrap();
        let contents = segments
            .into_iter()
            .flat_map(|segment| std::fs::read(segment).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(contents, b"qrstuvwxyz");
        let metadata = crate::log_storage::read_metadata(&path).unwrap().unwrap();
        assert!(metadata.truncated);
        assert_eq!(metadata.dropped_bytes, 16);
        assert_eq!(metadata.earliest_offset, 16);
        assert_eq!(metadata.retained_bytes, 10);
    }

    #[tokio::test]
    async fn pipe_writer_drains_reader_when_destination_cannot_be_opened() {
        let directory = tempfile::tempdir().unwrap();
        let (mut writer, reader) = tokio::io::duplex(128);
        let task = spawn_pipe_writer(reader, directory.path().to_path_buf(), Default::default());

        writer
            .write_all(b"output that must still be drained")
            .await
            .unwrap();
        writer.shutdown().await.unwrap();

        let error = finish_pipe_writer(task).await.unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::IsADirectory | std::io::ErrorKind::PermissionDenied
        ));
    }

    #[tokio::test]
    async fn finish_pipes_reports_the_first_stream_error() {
        let directory = tempfile::tempdir().unwrap();
        let (mut stdout_writer, stdout_reader) = tokio::io::duplex(64);
        let (mut stderr_writer, stderr_reader) = tokio::io::duplex(64);
        let stdout_task = spawn_pipe_writer(
            stdout_reader,
            directory.path().join("stdout.log"),
            Default::default(),
        );
        let stderr_task = spawn_pipe_writer(
            stderr_reader,
            directory.path().to_path_buf(),
            Default::default(),
        );

        stdout_writer.shutdown().await.unwrap();
        stderr_writer.shutdown().await.unwrap();
        let error = finish_pipes(stdout_task, stderr_task).await.unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::IsADirectory
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::Other
        ));
    }

    #[tokio::test]
    async fn finish_pipe_writer_converts_join_failures_to_io_errors() {
        let task = tokio::spawn(async {
            panic!("intentional test task failure");
            #[allow(unreachable_code)]
            Ok::<(), std::io::Error>(())
        });

        let error = finish_pipe_writer(task).await.unwrap_err();
        assert!(error.to_string().contains("process output task failed"));
    }
}
