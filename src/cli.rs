use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::domain::{Job, NewJob};
use crate::git::capture_submission;
use crate::service::Service;
use crate::{ServiceClient, StokerPaths, Store, is_service_unavailable};

#[derive(Debug, Parser)]
#[command(name = "stoker", about = "A local Git-aware job scheduler")]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    Submit(SubmitArgs),
    Show {
        id: Uuid,
    },
    Ps {
        #[arg(long)]
        user: Option<String>,
    },
    Serve,
    #[command(name = "service-run", hide = true)]
    ServiceRun,
    Status,
    Stop,
    Commit {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Logs {
        id: Uuid,
        #[arg(short = 'f', long)]
        follow: bool,
    },
}

#[derive(Debug, Args)]
pub struct SubmitArgs {
    #[arg(long)]
    pub user: String,
    #[arg(long)]
    pub name: String,
    #[arg(
        long = "cmd",
        required = true,
        num_args = 1..,
        allow_hyphen_values = true,
        help = "Command and arguments to execute (must be last)"
    )]
    pub command: Vec<String>,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run_command(cli.command)
}

pub fn run_command(command: CliCommand) -> anyhow::Result<()> {
    match command {
        CliCommand::Submit(args) => submit(args),
        CliCommand::Show { id } => show(id),
        CliCommand::Ps { user } => ps(user.as_deref()),
        CliCommand::Serve => serve(),
        CliCommand::ServiceRun => service_run(),
        CliCommand::Status => status(),
        CliCommand::Stop => stop(),
        CliCommand::Commit { id } => commit(id),
        CliCommand::Cancel { id } => cancel(id),
        CliCommand::Logs { id, follow } => logs(id, follow),
    }
}

fn open_store() -> anyhow::Result<Store> {
    let paths = StokerPaths::from_env()?;
    paths.ensure()?;
    Store::open(paths.database).context("open Stoker database")
}

fn open_paths() -> anyhow::Result<StokerPaths> {
    let paths = StokerPaths::from_env()?;
    paths.ensure()?;
    Ok(paths)
}

fn runtime() -> anyhow::Result<Runtime> {
    Runtime::new().context("create scheduler runtime")
}

fn terminate_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn serve() -> anyhow::Result<()> {
    let paths = open_paths()?;
    // An active endpoint is authoritative for the user-facing duplicate error;
    // stale endpoints are cleaned by the child after it acquires the lock.
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(_) => anyhow::bail!("scheduler service is already running"),
        Err(error) if is_service_unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.service_log())
        .context("open scheduler service log")?;
    let log_err = log.try_clone().context("duplicate scheduler service log")?;
    let executable = std::env::current_exe().context("locate stoker executable")?;
    let mut child = Command::new(executable)
        .arg("service-run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .context("start scheduler service")?;

    let startup_timeout = Duration::from_secs(5);
    let started = std::time::Instant::now();
    while started.elapsed() < startup_timeout {
        let status = match runtime() {
            Ok(runtime) => runtime.block_on(ServiceClient::new(paths.clone()).status()),
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };
        match status {
            Ok(_) => return Ok(()),
            Err(error) if is_service_unavailable(&error) => {}
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(anyhow::anyhow!(
                    "scheduler service exited during startup ({status})"
                ));
            }
            Ok(None) => {}
            Err(error) => {
                terminate_child(&mut child);
                return Err(error).context("check scheduler service");
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Do not leave a failed child behind if endpoint startup timed out.
    terminate_child(&mut child);
    anyhow::bail!("timed out waiting for scheduler service to start")
}

fn service_run() -> anyhow::Result<()> {
    let paths = open_paths()?;
    runtime()?.block_on(Service::new(paths)?.run())
}

fn status() -> anyhow::Result<()> {
    let paths = open_paths()?;
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(service) => {
            println!("Scheduler: running");
            println!("PID: {}", service.pid);
            println!(
                "Active job: {}",
                service
                    .active_job
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".into())
            );
            println!("Queued jobs: {}", service.queued_jobs);
        }
        Err(error) if is_service_unavailable(&error) => {
            let queued = Store::open(&paths.database)?
                .list_jobs(None)?
                .into_iter()
                .filter(|job| job.state == crate::JobState::Queued)
                .count();
            println!("Scheduler: stopped");
            println!("Queued jobs: {queued}");
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn stop() -> anyhow::Result<()> {
    let paths = open_paths()?;
    match runtime()?.block_on(ServiceClient::new(paths).stop()) {
        Ok(()) => Ok(()),
        Err(error) if is_service_unavailable(&error) => {
            anyhow::bail!("Scheduler is not running. Start it with 'stoker serve'.")
        }
        Err(error) => Err(error),
    }
}

fn commit(id: Uuid) -> anyhow::Result<()> {
    let paths = open_paths()?;
    runtime()?.block_on(ServiceClient::new(paths).commit(id))
}

fn cancel(id: Uuid) -> anyhow::Result<()> {
    let paths = open_paths()?;
    runtime()?.block_on(ServiceClient::new(paths).cancel(id))
}

fn logs(id: Uuid, follow: bool) -> anyhow::Result<()> {
    let paths = open_paths()?;
    if follow {
        return runtime()?.block_on(ServiceClient::new(paths).follow_logs(id));
    }
    let store = Store::open(paths.database)?;
    store.get_job(id)?;
    let run = paths.runs.join(id.to_string());
    let stdout = std::fs::read(run.join("stdout.log"))
        .with_context(|| format!("read stdout log for job {id}"))?;
    let stderr = std::fs::read(run.join("stderr.log"))
        .with_context(|| format!("read stderr log for job {id}"))?;
    use std::io::Write;
    std::io::stdout().write_all(&stdout)?;
    std::io::stderr().write_all(&stderr)?;
    Ok(())
}

fn submit(args: SubmitArgs) -> anyhow::Result<()> {
    if args.user.trim().is_empty() {
        anyhow::bail!("--user must not be empty");
    }
    if args.name.trim().is_empty() {
        anyhow::bail!("--name must not be empty");
    }
    if args.command.is_empty() {
        anyhow::bail!("at least one command element is required after `--cmd`");
    }

    let current_dir = std::env::current_dir().context("determine current directory")?;
    let snapshot = capture_submission(&current_dir).context("capture Git submission")?;
    let id = open_store()?.create_job(NewJob {
        name: args.name,
        user: args.user,
        repository: snapshot.repository,
        git_commit: snapshot.git_commit,
        cwd: snapshot.cwd,
        command: args.command,
    })?;
    println!("Created job {id} (DRAFT)");
    Ok(())
}

fn show(id: Uuid) -> anyhow::Result<()> {
    let job = open_store()?.get_job(id)?;
    print_job(&job);
    Ok(())
}

fn ps(owner: Option<&str>) -> anyhow::Result<()> {
    for job in open_store()?.list_jobs(owner)? {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            job.id,
            job.user,
            job.name,
            job.state,
            job.committed_at
                .or(Some(job.created_at))
                .map(|time| time.to_rfc3339())
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn print_job(job: &Job) {
    println!("id: {}", job.id);
    println!("name: {}", job.name);
    println!("user: {}", job.user);
    println!("repository: {}", job.repository.display());
    println!("git_commit: {}", job.git_commit);
    println!("cwd: {}", job.cwd.display());
    println!("command: {:?}", job.command);
    println!("state: {}", job.state);
    println!("created_at: {}", job.created_at.to_rfc3339());
    println!(
        "committed_at: {}",
        format_optional_time(job.committed_at.as_ref())
    );
    println!(
        "started_at: {}",
        format_optional_time(job.started_at.as_ref())
    );
    println!(
        "finished_at: {}",
        format_optional_time(job.finished_at.as_ref())
    );
    println!("exit_code: {:?}", job.exit_code);
    println!("pid: {:?}", job.pid);
    println!(
        "execution_dir: {}",
        job.execution_dir
            .as_deref()
            .unwrap_or(Path::new(""))
            .display()
    );
    println!("failure_detail: {:?}", job.failure_detail);
}

fn format_optional_time(value: Option<&chrono::DateTime<chrono::Utc>>) -> String {
    value
        .map(chrono::DateTime::to_rfc3339)
        .unwrap_or_else(|| "-".into())
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::terminate_child;
    use std::process::Command;

    #[test]
    fn terminate_child_kills_and_waits_for_startup_failure_cleanup() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("spawn test child");
        terminate_child(&mut child);
        assert!(child.try_wait().expect("check test child").is_some());
    }
}
