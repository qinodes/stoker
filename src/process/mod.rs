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
