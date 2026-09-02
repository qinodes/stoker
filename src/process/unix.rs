use std::io;
use std::process::{ExitStatus, Stdio};

use async_trait::async_trait;
use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::{Pid, setsid};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

use super::{ManagedProcess, ProcessSpec, finish_pipes, spawn_pipe_writer};

pub(crate) async fn spawn(spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SAFETY: setsid only affects this child before exec. No memory is
    // allocated or shared Rust state is accessed in the child.
    unsafe {
        command.pre_exec(|| {
            setsid()
                .map(|_| ())
                .map_err(|error| io::Error::from_raw_os_error(error as i32))
        });
    }

    let mut child = command.spawn()?;
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("spawned process has no PID"))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        let _ = child.start_kill();
        io::Error::other("spawned process has no stdout pipe")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        let _ = child.start_kill();
        io::Error::other("spawned process has no stderr pipe")
    })?;
    let stdout_task = spawn_pipe_writer(stdout, spec.stdout_log);
    let stderr_task = spawn_pipe_writer(stderr, spec.stderr_log);

    Ok(Box::new(UnixManagedProcess {
        pid,
        group_id: pid,
        child,
        stdout_task: Some(stdout_task),
        stderr_task: Some(stderr_task),
        terminated: false,
    }))
}

struct UnixManagedProcess {
    pid: u32,
    group_id: u32,
    child: Child,
    stdout_task: Option<JoinHandle<io::Result<()>>>,
    stderr_task: Option<JoinHandle<io::Result<()>>>,
    terminated: bool,
}

#[async_trait]
impl ManagedProcess for UnixManagedProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
        let mut process = self;
        process.wait_inner().await
    }

    async fn wait_with_cancel(
        self: Box<Self>,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> io::Result<ExitStatus> {
        let mut process = self;
        tokio::select! {
            status = process.wait_inner() => status,
            _ = &mut cancel => {
                process.terminate_tree().await?;
                process.wait_inner().await
            }
        }
    }

    async fn terminate_tree(&mut self) -> io::Result<()> {
        if self.terminated || self.group_id == 0 {
            return Ok(());
        }
        match killpg(Pid::from_raw(self.group_id as i32), Signal::SIGTERM) {
            Ok(()) | Err(Errno::ESRCH) => {
                self.terminated = true;
                // Give a well-behaved process tree a short grace period, then
                // escalate for commands that ignore or trap SIGTERM. The
                // process group is used for both signals so descendants are
                // covered by the same bounded cancellation path.
                for _ in 0..50 {
                    match killpg(Pid::from_raw(self.group_id as i32), None) {
                        Ok(()) => sleep(Duration::from_millis(10)).await,
                        Err(Errno::ESRCH) => return Ok(()),
                        Err(error) => {
                            return Err(io::Error::from_raw_os_error(error as i32));
                        }
                    }
                }
                match killpg(Pid::from_raw(self.group_id as i32), Signal::SIGKILL) {
                    Ok(()) | Err(Errno::ESRCH) => Ok(()),
                    Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
                }
            }
            Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
}

impl UnixManagedProcess {
    async fn wait_inner(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        let stdout_task = self
            .stdout_task
            .take()
            .ok_or_else(|| io::Error::other("stdout task already joined"))?;
        let stderr_task = self
            .stderr_task
            .take()
            .ok_or_else(|| io::Error::other("stderr task already joined"))?;
        finish_pipes(stdout_task, stderr_task).await?;
        wait_for_group_exit(self.group_id).await?;
        Ok(status)
    }
}

async fn wait_for_group_exit(group_id: u32) -> io::Result<()> {
    if group_id == 0 {
        return Ok(());
    }
    loop {
        match killpg(Pid::from_raw(group_id as i32), None) {
            Ok(()) => sleep(Duration::from_millis(10)).await,
            Err(Errno::ESRCH) => return Ok(()),
            Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
}
