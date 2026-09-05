use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// The immutable inputs needed to start one managed process.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
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
            unix::spawn(spec).await
        }
        #[cfg(windows)]
        {
            windows::spawn(spec).await
        }
    }
}

/// A descriptive alias for callers that prefer to name the concrete system
/// implementation rather than the controller trait.
pub type SystemProcessController = DefaultProcessController;

pub(crate) fn spawn_pipe_writer<R>(reader: R, path: PathBuf) -> JoinHandle<io::Result<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
        {
            Ok(file) => Some(file),
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
                if let Some(file) = file.as_mut()
                    && let Err(error) = file.flush().await
                {
                    write_error.get_or_insert(error);
                }
                return write_error.map_or(Ok(()), Err);
            }
            if let Some(output) = file.as_mut() {
                let result = output.write_all(&buffer[..bytes_read]).await;
                if let Err(error) = result {
                    write_error.get_or_insert(error);
                    file = None;
                }
            }
        }
    })
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
        DefaultProcessController, ManagedProcess, finish_pipe_writer, finish_pipes,
        spawn_pipe_writer,
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
        let task = spawn_pipe_writer(reader, path.clone());

        writer.write_all(b"first\n").await.unwrap();
        writer.write_all(b"second\n").await.unwrap();
        writer.shutdown().await.unwrap();

        finish_pipe_writer(task).await.unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "first\nsecond\n");
    }

    #[tokio::test]
    async fn pipe_writer_drains_reader_when_destination_cannot_be_opened() {
        let directory = tempfile::tempdir().unwrap();
        let (mut writer, reader) = tokio::io::duplex(128);
        let task = spawn_pipe_writer(reader, directory.path().to_path_buf());

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
        let stdout_task = spawn_pipe_writer(stdout_reader, directory.path().join("stdout.log"));
        let stderr_task = spawn_pipe_writer(stderr_reader, directory.path().to_path_buf());

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
