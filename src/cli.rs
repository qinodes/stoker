use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::config::normalize_path;
use crate::domain::{Job, JobState, NewJob};
use crate::service::Service;
use crate::{ServiceClient, StokerPaths, Store, is_service_unavailable};

#[derive(Debug, Parser)]
#[command(
    name = "stoker",
    about = "A local job scheduler",
    version = env!("CARGO_PKG_VERSION")
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    Add(AddArgs),
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
    #[command(about = "Remove terminal job records and logs")]
    Clean,
    #[command(about = "Update stoker from GitHub Releases")]
    Update,
    #[command(about = "Uninstall stoker with confirmation")]
    Uninstall,
    Start,
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
pub struct AddArgs {
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
        CliCommand::Add(args) => add(args),
        CliCommand::Show { id } => show(id),
        CliCommand::Jobs { user, state } => jobs(user.as_deref(), state),
        CliCommand::Clean => clean(),
        CliCommand::Update => update(),
        CliCommand::Uninstall => uninstall(),
        CliCommand::Start => start(),
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

fn start() -> anyhow::Result<()> {
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
            anyhow::bail!("Scheduler is not running. Start it with 'stoker start'.")
        }
        Err(error) => Err(error),
    }
}

fn update() -> anyhow::Result<()> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).context("parse current Stoker version")?;
    let release = latest_release()?;
    let latest = release.version()?;
    match latest.cmp(&current) {
        std::cmp::Ordering::Less => {
            anyhow::bail!(
                "GitHub reports Stoker {latest}, which is older than the installed {current}; refusing to downgrade"
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

    ensure_scheduler_stopped()?;
    let current_exe = std::env::current_exe().context("locate current Stoker executable")?;
    let binary = download_release_binary(&release)?;
    install_updated_binary(&current_exe, &binary)?;
    #[cfg(unix)]
    println!("Stoker was updated to {latest}.");
    #[cfg(windows)]
    println!("Stoker update to {latest} has been scheduled.");
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

    let current_exe = std::env::current_exe().context("locate current Stoker executable")?;

    #[cfg(unix)]
    return remove_unix_binary(&current_exe);

    #[cfg(windows)]
    return schedule_windows_uninstall(std::process::id(), &current_exe);
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

impl GithubRelease {
    fn version(&self) -> anyhow::Result<Version> {
        let version = self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name);
        Version::parse(version).context("parse latest GitHub release version")
    }
}

fn latest_release() -> anyhow::Result<GithubRelease> {
    let repository = env!("CARGO_PKG_REPOSITORY")
        .strip_prefix("https://github.com/")
        .and_then(|repository| repository.strip_suffix(".git").or(Some(repository)))
        .filter(|repository| !repository.is_empty())
        .ok_or_else(|| anyhow::anyhow!("package repository is not a GitHub repository"))?;
    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let mut response = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .context("request latest Stoker GitHub release")?;
    let body = response
        .body_mut()
        .read_to_vec()
        .context("read latest Stoker GitHub release")?;
    serde_json::from_slice(&body).context("parse latest Stoker GitHub release")
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-windows-x86_64.exe")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-linux-x86_64")
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-macos-arm64")
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-macos-x86_64")
}

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64")
)))]
fn platform_binary_name() -> anyhow::Result<&'static str> {
    anyhow::bail!("automatic updates are not supported on this platform")
}

fn release_asset<'a>(release: &'a GithubRelease, name: &str) -> anyhow::Result<&'a GithubAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| anyhow::anyhow!("GitHub release does not contain asset {name}"))
}

fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("download {url}"))?;
    response
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("read downloaded content from {url}"))
}

fn checksum_for(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let checksum = fields.next()?.trim_start_matches('*');
        let name = fields.next()?.rsplit('/').next()?;
        (name == asset_name).then(|| checksum.to_ascii_lowercase())
    })
}

fn download_release_binary(release: &GithubRelease) -> anyhow::Result<Vec<u8>> {
    let binary_name = platform_binary_name()?;
    let binary_asset = release_asset(release, binary_name)?;
    let checksum_asset = release_asset(release, "SHA256SUMS")?;
    let binary = download_bytes(&binary_asset.browser_download_url)?;
    let checksums = String::from_utf8(download_bytes(&checksum_asset.browser_download_url)?)
        .context("decode SHA256SUMS")?;
    let expected = checksum_for(&checksums, binary_name)
        .ok_or_else(|| anyhow::anyhow!("SHA256SUMS does not contain {binary_name}"))?;
    let actual = Sha256::digest(&binary)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        anyhow::bail!("SHA-256 mismatch for {binary_name}");
    }
    Ok(binary)
}

fn ensure_scheduler_stopped() -> anyhow::Result<()> {
    let paths = StokerPaths::from_env()?;
    match runtime()?.block_on(ServiceClient::new(paths).status()) {
        Ok(_) => anyhow::bail!("Scheduler is running. Stop it with 'stoker stop' before updating."),
        Err(error) if is_service_unavailable(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn install_updated_binary(current_exe: &Path, binary: &[u8]) -> anyhow::Result<()> {
    let update_dir = std::env::temp_dir().join(format!("stoker-update-{}", Uuid::new_v4()));
    fs::create_dir(&update_dir).context("create Stoker update directory")?;
    let update_binary = update_dir.join(platform_binary_name()?);
    if let Err(error) = fs::write(&update_binary, binary) {
        let _ = fs::remove_dir_all(&update_dir);
        return Err(error).context("write downloaded Stoker binary");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::metadata(current_exe)
            .context("read current Stoker executable permissions")?
            .permissions();
        fs::set_permissions(
            &update_binary,
            fs::Permissions::from_mode(permissions.mode()),
        )
        .context("set updated Stoker executable permissions")?;
        if let Err(error) = fs::rename(&update_binary, current_exe) {
            let _ = fs::remove_dir_all(&update_dir);
            return Err(error).context("replace current Stoker executable");
        }
        fs::remove_dir_all(&update_dir).context("remove Stoker update directory")?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        schedule_windows_update(std::process::id(), current_exe, &update_binary, &update_dir)?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        let _ = fs::remove_dir_all(&update_dir);
        anyhow::bail!("updating is not supported on this platform")
    }
}

#[cfg(unix)]
fn remove_unix_binary(executable: &Path) -> anyhow::Result<()> {
    fs::remove_file(executable)
        .with_context(|| format!("remove Stoker executable at {}", executable.display()))?;
    println!("Stoker has been uninstalled. Job data and logs were kept.");
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

fn is_confirmation(response: &str) -> bool {
    matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(windows)]
fn schedule_windows_uninstall(process_id: u32, executable: &Path) -> anyhow::Result<()> {
    let script = std::env::temp_dir().join(format!("stoker-uninstall-{}.cmd", Uuid::new_v4()));
    fs::write(&script, windows_uninstall_script(process_id, executable))
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
fn schedule_windows_update(
    process_id: u32,
    executable: &Path,
    update_binary: &Path,
    update_dir: &Path,
) -> anyhow::Result<()> {
    let script = std::env::temp_dir().join(format!("stoker-update-{}.cmd", Uuid::new_v4()));
    fs::write(
        &script,
        windows_update_script(process_id, executable, update_binary, update_dir),
    )
    .context("create Windows update helper")?;
    let command = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    Command::new(command)
        .args(["/C", script.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .spawn()
        .context("schedule Windows update helper")?;
    Ok(())
}

#[cfg(windows)]
fn windows_uninstall_script(process_id: u32, executable: &Path) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\ndel /F /Q \"{}\"\r\ndel \"%~f0\"\r\n",
        executable.display()
    )
}

#[cfg(windows)]
fn windows_update_script(
    process_id: u32,
    executable: &Path,
    update_binary: &Path,
    update_dir: &Path,
) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\nmove /Y \"{}\" \"{}\" >NUL\r\nif errorlevel 1 (\r\n  echo Failed to replace Stoker executable.\r\n  exit /b 1\r\n)\r\nrmdir /S /Q \"{}\"\r\ndel \"%~f0\"\r\n",
        update_binary.display(),
        executable.display(),
        update_dir.display()
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

fn add(args: AddArgs) -> anyhow::Result<()> {
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
    let cwd = normalize_path(
        current_dir
            .canonicalize()
            .context("resolve working directory")?,
    );
    let id = open_store()?.create_job(NewJob {
        name: args.name,
        user: args.user,
        cwd,
        command: args.command,
    })?;
    println!("Created job {id} (DRAFT)");
    Ok(())
}

fn show(id: Uuid) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let job = Store::open(&paths.database)?.get_job(id)?;
    print_job(&job);
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

fn clean() -> anyhow::Result<()> {
    let paths = open_paths()?;
    let jobs = Store::open(&paths.database)?.clean_terminal_jobs()?;
    for job in &jobs {
        let run_dir = paths.runs.join(job.id.to_string());
        if run_dir.exists() {
            fs::remove_dir_all(&run_dir)
                .with_context(|| format!("remove logs for job {}", job.id))?;
        }
    }
    println!("Cleaned {} terminal job(s).", jobs.len());
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

fn print_job(job: &Job) {
    let execution_cwd = &job.cwd;
    let execution_cwd_status = match job.state {
        JobState::Draft | JobState::Queued => "planned",
        JobState::Starting | JobState::Running | JobState::Cancelling => "active",
        _ => "source directory retained",
    };
    println!("id: {}", job.id);
    println!("name: {}", job.name);
    println!("user: {}", job.user);
    println!("cwd: {}", job.cwd.display());
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
    use super::{Cli, checksum_for, is_confirmation, platform_binary_name};
    use clap::Parser;

    #[test]
    fn platform_binary_name_matches_release_assets() {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-windows-x86_64.exe");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-linux-x86_64");
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-macos-arm64");
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-macos-x86_64");
    }

    #[test]
    fn parses_checksum_for_the_exact_asset_name() {
        let checksums = "abc123  stoker-linux-x86_64\ndef456  stoker-linux-x86_64-extra";
        assert_eq!(
            checksum_for(checksums, "stoker-linux-x86_64").as_deref(),
            Some("abc123")
        );
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
    fn uninstall_is_a_valid_cli_command() {
        assert!(Cli::try_parse_from(["stoker", "uninstall"]).is_ok());
    }
}

#[cfg(all(test, windows))]
mod windows_uninstall_tests {
    use super::{windows_uninstall_script, windows_update_script};
    use std::path::Path;

    #[test]
    fn uninstall_helper_waits_for_stoker_then_removes_the_binary() {
        let script = windows_uninstall_script(1234, Path::new(r"C:\Tools\stoker.exe"));
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("del /F /Q \"C:\\Tools\\stoker.exe\""));
    }

    #[test]
    fn update_helper_waits_for_stoker_then_moves_the_downloaded_binary() {
        let script = windows_update_script(
            1234,
            Path::new(r"C:\Tools\stoker.exe"),
            Path::new(r"C:\Temp\stoker.exe"),
            Path::new(r"C:\Temp\stoker-update"),
        );
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("move /Y \"C:\\Temp\\stoker.exe\" \"C:\\Tools\\stoker.exe\""));
    }
}
