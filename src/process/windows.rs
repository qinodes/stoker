use std::ffi::{OsStr, OsString};
use std::io;
use std::mem::zeroed;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};

use async_trait::async_trait;
use tokio::fs::File as TokioFile;
use tokio::task::JoinHandle;
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};

use super::{ManagedProcess, ProcessSpec, finish_pipes, spawn_pipe_writer};

pub(crate) async fn spawn(spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
    // Keep the primary thread suspended until the process has been assigned to
    // the Job Object, closing the escape race during process startup.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }

    let (stdout_read, stdout_write) = match create_output_pipe() {
        Ok(pipe) => pipe,
        Err(error) => {
            unsafe { CloseHandle(job) };
            return Err(error);
        }
    };
    let (stderr_read, stderr_write) = match create_output_pipe() {
        Ok(pipe) => pipe,
        Err(error) => {
            unsafe {
                CloseHandle(stdout_read);
                CloseHandle(stdout_write);
                CloseHandle(job);
            }
            return Err(error);
        }
    };

    let mut command_line = command_line(&spec.program, &spec.args);
    let application_name = wide_path(&spec.program);
    let current_directory = wide_path(spec.cwd.as_os_str());
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdOutput = stdout_write;
    startup.hStdError = stderr_write;
    startup.hStdInput = null_mut();
    let mut information: PROCESS_INFORMATION = unsafe { zeroed() };

    let created = unsafe {
        CreateProcessW(
            application_name.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED,
            null(),
            current_directory.as_ptr(),
            &startup,
            &mut information,
        )
    };
    unsafe {
        CloseHandle(stdout_write);
        CloseHandle(stderr_write);
    }
    if created == 0 {
        unsafe {
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
            CloseHandle(job);
        }
        return Err(io::Error::last_os_error());
    }

    let process = information.hProcess;
    let thread = information.hThread;
    if unsafe { AssignProcessToJobObject(job, process) } == 0 {
        let error = io::Error::last_os_error();
        unsafe {
            TerminateProcess(process, 1);
            CloseHandle(thread);
            CloseHandle(process);
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
            CloseHandle(job);
        }
        return Err(error);
    }
    if unsafe { ResumeThread(thread) } == u32::MAX {
        let error = io::Error::last_os_error();
        unsafe {
            TerminateJobObject(job, 1);
            CloseHandle(thread);
            CloseHandle(process);
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
            CloseHandle(job);
        }
        return Err(error);
    }
    unsafe { CloseHandle(thread) };

    let stdout =
        unsafe { TokioFile::from_std(std::fs::File::from_raw_handle(stdout_read as RawHandle)) };
    let stderr =
        unsafe { TokioFile::from_std(std::fs::File::from_raw_handle(stderr_read as RawHandle)) };
    let stdout_task = spawn_pipe_writer(stdout, spec.stdout_log);
    let stderr_task = spawn_pipe_writer(stderr, spec.stderr_log);

    Ok(Box::new(WindowsManagedProcess {
        pid: information.dwProcessId,
        process: process as usize,
        job: job as usize,
        stdout_task: Some(stdout_task),
        stderr_task: Some(stderr_task),
        terminated: false,
    }))
}

fn create_output_pipe() -> io::Result<(HANDLE, HANDLE)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) } == 0 {
        let error = io::Error::last_os_error();
        unsafe {
            CloseHandle(read);
            CloseHandle(write);
        }
        return Err(error);
    }
    Ok((read, write))
}

fn wide_path(path: &OsStr) -> Vec<u16> {
    path.encode_wide().chain(std::iter::once(0)).collect()
}

fn command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut line = quote_arg(program);
    for arg in args {
        line.push(b' ' as u16);
        line.extend(quote_arg(arg));
    }
    line.push(0);
    line
}

// Quote according to the CommandLineToArgvW/CRT backslash-and-quote rules.
fn quote_arg(value: &OsStr) -> Vec<u16> {
    let raw: Vec<u16> = value.encode_wide().collect();
    if !raw.is_empty()
        && raw.iter().all(|character| {
            *character != b' ' as u16 && *character != b'\t' as u16 && *character != b'"' as u16
        })
    {
        return raw;
    }
    let mut result = vec![b'"' as u16];
    let mut backslashes = 0;
    for character in raw {
        if character == b'\\' as u16 {
            backslashes += 1;
        } else if character == b'"' as u16 {
            result.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            result.push(character);
            backslashes = 0;
        } else {
            result.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
            result.push(character);
            backslashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    result.push(b'"' as u16);
    result
}

struct WindowsManagedProcess {
    pid: u32,
    process: usize,
    job: usize,
    stdout_task: Option<JoinHandle<io::Result<()>>>,
    stderr_task: Option<JoinHandle<io::Result<()>>>,
    terminated: bool,
}

#[async_trait]
impl ManagedProcess for WindowsManagedProcess {
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
        if self.terminated {
            return Ok(());
        }
        if unsafe { TerminateJobObject(self.job as HANDLE, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.terminated = true;
        Ok(())
    }
}

impl WindowsManagedProcess {
    async fn wait_inner(&mut self) -> io::Result<ExitStatus> {
        let process = self.process;
        let job = self.job;
        let status_task = tokio::task::spawn_blocking(move || unsafe {
            let result = WaitForSingleObject(job as HANDLE, INFINITE);
            if result == WAIT_FAILED {
                return Err(io::Error::last_os_error());
            }
            if result != WAIT_OBJECT_0 {
                return Err(io::Error::other("unexpected Job Object wait result"));
            }
            let mut code = 1;
            if GetExitCodeProcess(process as HANDLE, &mut code) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(ExitStatus::from_raw(code))
        });
        let status = status_task
            .await
            .map_err(|error| io::Error::other(format!("process wait task failed: {error}")))?;
        let stdout_task = self
            .stdout_task
            .take()
            .ok_or_else(|| io::Error::other("stdout task already joined"))?;
        let stderr_task = self
            .stderr_task
            .take()
            .ok_or_else(|| io::Error::other("stderr task already joined"))?;
        let pipes_result = finish_pipes(stdout_task, stderr_task).await;
        unsafe {
            CloseHandle(self.process as HANDLE);
            CloseHandle(self.job as HANDLE);
        }
        let status = status?;
        pipes_result?;
        Ok(status)
    }
}
