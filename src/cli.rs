use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use semver::Version;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::domain::{Job, JobState, NewJob};
use crate::git::capture_submission;
use crate::service::Service;
use crate::{ServiceClient, StokerPaths, Store, is_service_unavailable};

#[derive(Debug, Parser)]
#[command(
    name = "stoker",
    about = "A local Git-aware job scheduler",
    version = env!("CARGO_PKG_VERSION")
)]
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
    #[command(about = "List submitted jobs and their job IDs")]
    Jobs {
        #[arg(long)]
        user: Option<String>,
        #[arg(long, value_parser = parse_job_state)]
        state: Option<JobState>,
    },
    #[command(about = "Update stoker from crates.io")]
    Update,
    #[command(about = "Uninstall stoker with confirmation")]
    Uninstall,
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
        CliCommand::Jobs { user, state } => jobs(user.as_deref(), state),
        CliCommand::Update => update(),
        CliCommand::Uninstall => uninstall(),
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

fn update() -> anyhow::Result<()> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).context("parse current Stoker version")?;
    let latest = published_version()?;
    match latest.cmp(&current) {
        std::cmp::Ordering::Less => {
            anyhow::bail!(
                "crates.io reports stoker-engine {latest}, which is older than the installed {current}; refusing to downgrade"
            );
        }
        std::cmp::Ordering::Equal => {
            println!("Stoker is already up to date ({current}).");
            return Ok(());
        }
        std::cmp::Ordering::Greater => {}
    }

    println!("Stoker will update from {current} to {latest}.");
    if !request_confirmation("Continue with update")? {
        println!("Update cancelled.");
        return Ok(());
    }

    let status = Command::new("cargo")
        .args(cargo_install_args(&latest))
        .status()
        .context("run Cargo to update stoker")?;
    if !status.success() {
        anyhow::bail!("Cargo could not update stoker (exit status: {status})");
    }
    Ok(())
}

fn uninstall() -> anyhow::Result<()> {
    let paths = StokerPaths::from_env()?;
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(_) => {
            anyhow::bail!("Scheduler is running. Stop it with 'stoker stop' before uninstalling.")
        }
        Err(error) if is_service_unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    println!("Stoker will be uninstalled.");
    println!(
        "Job data and logs will be kept at {}.",
        paths.root.display()
    );
    if !request_confirmation("Continue with uninstall")? {
        println!("Uninstall cancelled.");
        return Ok(());
    }

    #[cfg(unix)]
    return run_cargo_uninstall();

    #[cfg(windows)]
    return schedule_windows_uninstall();
}

fn published_version() -> anyhow::Result<Version> {
    let output = Command::new("cargo")
        .args(["search", env!("CARGO_PKG_NAME"), "--limit", "1"])
        .output()
        .context("run Cargo to look up the latest stoker version")?;
    if !output.status.success() {
        anyhow::bail!(
            "Cargo could not look up the latest Stoker version (exit status: {})",
            output.status
        );
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let version = parse_published_version(&output).ok_or_else(|| {
        anyhow::anyhow!(
            "Cargo did not return a version for {}",
            env!("CARGO_PKG_NAME")
        )
    })?;
    Version::parse(&version).context("parse latest Stoker version from Cargo")
}

fn cargo_install_args(version: &Version) -> Vec<String> {
    vec![
        "install".into(),
        env!("CARGO_PKG_NAME").into(),
        "--version".into(),
        version.to_string(),
        "--locked".into(),
        "--force".into(),
    ]
}

fn cargo_uninstall_args() -> [&'static str; 2] {
    ["uninstall", env!("CARGO_PKG_NAME")]
}

#[cfg(unix)]
fn run_cargo_uninstall() -> anyhow::Result<()> {
    let status = Command::new("cargo")
        .args(cargo_uninstall_args())
        .status()
        .context("run Cargo to uninstall stoker")?;
    if !status.success() {
        anyhow::bail!("Cargo could not uninstall stoker (exit status: {status})");
    }
    Ok(())
}

fn request_confirmation(action: &str) -> anyhow::Result<bool> {
    print!("{action}? [y/N]: ");
    io::stdout().flush().context("write confirmation prompt")?;
    let mut response = String::new();
    io::stdin()
        .read_line(&mut response)
        .context("read confirmation")?;
    Ok(is_confirmation(&response))
}

fn parse_published_version(output: &str) -> Option<String> {
    let prefix = format!("{} = \"", env!("CARGO_PKG_NAME"));
    output.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix(&prefix)?
            .split('"')
            .next()
            .filter(|version| !version.is_empty())
            .map(str::to_owned)
    })
}

fn is_confirmation(response: &str) -> bool {
    matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(windows)]
fn schedule_windows_uninstall() -> anyhow::Result<()> {
    let script = std::env::temp_dir().join(format!("stoker-uninstall-{}.cmd", Uuid::new_v4()));
    std::fs::write(&script, windows_uninstall_script(std::process::id()))
        .context("create Windows uninstall helper")?;
    let command = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    Command::new(command)
        .args(["/C", script.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .spawn()
        .context("schedule Windows uninstall helper")?;
    println!("Uninstall scheduled. It will run after Stoker exits.");
    Ok(())
}

#[cfg(windows)]
fn windows_uninstall_script(process_id: u32) -> String {
    let cargo_uninstall = cargo_uninstall_args().join(" ");
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\ncargo {cargo_uninstall}\r\ndel \"%~f0\"\r\n"
    )
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
    let paths = open_paths()?;
    let job = Store::open(&paths.database)?.get_job(id)?;
    print_job(&job, &paths);
    Ok(())
}

fn jobs(owner: Option<&str>, state: Option<JobState>) -> anyhow::Result<()> {
    let rows: Vec<_> = open_store()?
        .list_jobs_with_state(owner, state)?
        .into_iter()
        .map(|job| {
            [
                job.queue_order
                    .map(|order| order.to_string())
                    .unwrap_or_else(|| "-".into()),
                job.id.to_string(),
                job.user,
                job.name,
                job.state.to_string(),
                job.committed_at
                    .or(Some(job.created_at))
                    .map(|time| time.to_rfc3339())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let headers = ["queue_order", "job_id", "owner", "name", "state", "time"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    println!("{}", format_jobs_row(headers, &widths));
    for row in &rows {
        println!(
            "{}",
            format_jobs_row(
                [&row[0], &row[1], &row[2], &row[3], &row[4], &row[5]],
                &widths,
            )
        );
    }
    Ok(())
}

fn format_jobs_row(columns: [&str; 6], widths: &[usize; 6]) -> String {
    columns
        .into_iter()
        .zip(widths)
        .map(|(value, width)| format!("{value:<width$}", width = *width))
        .collect::<Vec<_>>()
        .join("  ")
}

fn parse_job_state(value: &str) -> Result<JobState, String> {
    value.to_ascii_uppercase().parse()
}

fn print_job(job: &Job, paths: &StokerPaths) {
    let source_cwd = job.repository.join(&job.cwd);
    let execution_dir = job
        .execution_dir
        .clone()
        .unwrap_or_else(|| paths.runs.join(job.id.to_string()).join("repo"));
    let execution_cwd = execution_dir.join(&job.cwd);
    let execution_cwd_status = match job.state {
        JobState::Draft | JobState::Queued => "planned",
        JobState::Starting | JobState::Running | JobState::Cancelling => "active",
        _ if job.execution_dir.is_some() => "retained after incomplete cleanup",
        _ => "cleaned after completion",
    };
    println!("id: {}", job.id);
    println!("name: {}", job.name);
    println!("user: {}", job.user);
    println!("repository: {}", job.repository.display());
    println!("git_commit: {}", job.git_commit);
    println!("cwd: {}", job.cwd.display());
    println!("source_cwd: {}", source_cwd.display());
    println!("execution_cwd: {}", execution_cwd.display());
    println!("execution_cwd_status: {execution_cwd_status}");
    println!("command: {:?}", job.command);
    println!("state: {}", job.state);
    println!(
        "queue_order: {}",
        job.queue_order
            .map(|order| order.to_string())
            .unwrap_or_else(|| "-".into())
    );
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

#[cfg(test)]
mod update_tests {
    use super::{
        Cli, cargo_install_args, cargo_uninstall_args, is_confirmation, parse_published_version,
    };
    use clap::Parser;
    use semver::Version;

    #[test]
    fn update_targets_the_published_package() {
        assert_eq!(
            cargo_install_args(&Version::parse("1.2.3").unwrap()),
            [
                "install",
                "stoker-engine",
                "--version",
                "1.2.3",
                "--locked",
                "--force"
            ]
        );
    }

    #[test]
    fn parses_the_exact_package_version_from_cargo_search_output() {
        let output = "stoker-engine = \"1.2.3\"    # scheduler\nstoker-engine-extra = \"9.9.9\"";
        assert_eq!(parse_published_version(output).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn update_confirmation_only_accepts_explicit_yes() {
        assert!(is_confirmation("y\n"));
        assert!(is_confirmation(" YES "));
        assert!(!is_confirmation(""));
        assert!(!is_confirmation("n"));
        assert!(!is_confirmation("anything else"));
    }

    #[test]
    fn uninstall_targets_the_published_package() {
        assert_eq!(cargo_uninstall_args(), ["uninstall", "stoker-engine"]);
    }

    #[test]
    fn uninstall_is_a_valid_cli_command() {
        assert!(Cli::try_parse_from(["stoker", "uninstall"]).is_ok());
    }
}

#[cfg(all(test, windows))]
mod windows_uninstall_tests {
    use super::windows_uninstall_script;

    #[test]
    fn uninstall_helper_waits_for_stoker_then_uses_cargo() {
        let script = windows_uninstall_script(1234);
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("cargo uninstall stoker-engine"));
    }
}
