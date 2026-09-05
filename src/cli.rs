use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::config::{ResolvedTimezone, TimezoneSource, normalize_path, resolve_timezone};
use crate::domain::{Job, JobState, NewJob};
use crate::ipc::StaleQueueMoveError;
use crate::queue_editor::{self, EditorMoveError};
use crate::service::Service;
use crate::{ServiceClient, StokerPaths, Store, StoreError, is_service_unavailable};

#[derive(Debug, Parser)]
#[command(
    name = "stoker",
    about = "A local job scheduler",
    version = env!("CARGO_PKG_VERSION")
)]
pub struct Cli {
    #[arg(
        long = "timezone",
        visible_alias = "tz",
        global = true,
        help = "Timezone used when displaying timestamps"
    )]
    pub timezone: Option<String>,
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    #[command(about = "Create a DRAFT job")]
    Add(AddArgs),
    #[command(about = "Show a job's details")]
    Show {
        #[arg(help = "Job ID")]
        id: Uuid,
    },
    #[command(about = "List submitted jobs and their job IDs")]
    Jobs {
        #[arg(long, help = "Filter by logical job owner")]
        user: Option<String>,
        #[arg(long, value_parser = parse_job_state, help = "Filter by job state")]
        state: Option<JobState>,
    },
    #[command(about = "Manage Stoker user configuration")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Remove terminal job records and logs")]
    Clean,
    #[command(about = "Update stoker from GitHub Releases")]
    Update(ConfirmationArgs),
    #[command(about = "Uninstall stoker with confirmation")]
    Uninstall(ConfirmationArgs),
    #[command(about = "Start the scheduler service")]
    Start,
    #[command(name = "service-run", hide = true)]
    ServiceRun,
    #[command(about = "Show scheduler, queue, and timezone status")]
    Status,
    #[command(about = "Lock, edit, or unlock the queue")]
    Queue {
        #[command(subcommand)]
        command: QueueCommand,
    },
    #[command(about = "Stop the scheduler service")]
    Stop(ConfirmationArgs),
    #[command(about = "Commit a DRAFT job to the queue")]
    Commit {
        #[arg(
            required_unless_present = "all",
            conflicts_with = "all",
            help = "Job ID"
        )]
        id: Option<Uuid>,
        #[arg(long, help = "Commit all DRAFT jobs in creation order")]
        all: bool,
    },
    #[command(about = "Pause all queued jobs")]
    Pause,
    #[command(about = "Resume paused jobs")]
    Resume,
    #[command(about = "Cancel a job")]
    Cancel(CancelArgs),
    #[command(about = "Show a job's logs")]
    Logs {
        #[arg(help = "Job ID")]
        id: Uuid,
        #[arg(short = 'f', long, help = "Follow new log output")]
        follow: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    #[command(about = "Set a configuration value")]
    Set {
        #[arg(value_enum)]
        key: ConfigKey,
        value: String,
    },
    #[command(about = "Get a configuration value")]
    Get {
        #[arg(value_enum)]
        key: ConfigKey,
    },
    #[command(about = "Clear a configuration value")]
    Unset {
        #[arg(value_enum)]
        key: ConfigKey,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigKey {
    Timezone,
}

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    #[command(about = "Prevent the scheduler from claiming queued jobs")]
    Lock,
    #[command(about = "Interactively reorder queued jobs")]
    Edit,
    #[command(about = "Allow the scheduler to claim queued jobs")]
    Unlock,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long, help = "Logical job owner label")]
    pub user: String,
    #[arg(long, help = "Job name")]
    pub name: String,
    #[arg(
        long = "cmd",
        required = true,
        allow_hyphen_values = true,
        help = "Complete shell command string (quote it when it contains spaces)"
    )]
    pub command: String,
}

#[derive(Debug, Args)]
pub struct ConfirmationArgs {
    #[arg(long, help = "Skip the confirmation prompt")]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct CancelArgs {
    pub id: Uuid,
    #[command(flatten)]
    pub confirmation: ConfirmationArgs,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = StokerPaths::from_env()?;
    paths.ensure()?;
    run_command_with_timezone(cli.command, cli.timezone)
}

pub fn run_command(command: CliCommand) -> anyhow::Result<()> {
    run_command_with_timezone(command, None)
}

fn run_command_with_timezone(command: CliCommand, timezone: Option<String>) -> anyhow::Result<()> {
    match command {
        CliCommand::Add(args) => add(args),
        CliCommand::Show { id } => show(id, timezone.as_deref()),
        CliCommand::Jobs { user, state } => jobs(user.as_deref(), state, timezone.as_deref()),
        CliCommand::Config { command } => config(command),
        CliCommand::Clean => clean(),
        CliCommand::Update(args) => update(args.yes),
        CliCommand::Uninstall(args) => uninstall(args.yes),
        CliCommand::Start => start(),
        CliCommand::ServiceRun => service_run(),
        CliCommand::Status => status(),
        CliCommand::Queue { command } => match command {
            QueueCommand::Lock => lock_queue(),
            QueueCommand::Edit => queue_edit(),
            QueueCommand::Unlock => unlock_queue(),
        },
        CliCommand::Stop(args) => stop(args.yes),
        CliCommand::Commit { id, all } => commit(id, all),
        CliCommand::Pause => pause(),
        CliCommand::Resume => resume(),
        CliCommand::Cancel(args) => cancel(args.id, args.confirmation.yes),
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

fn config(command: ConfigCommand) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let mut current = paths.read_config()?;
    match command {
        ConfigCommand::Set {
            key: ConfigKey::Timezone,
            value,
        } => {
            resolve_timezone(&paths, Some(&value))?;
            current.timezone = Some(value.clone());
            paths.write_config(&current)?;
            println!("Set timezone to {value}.");
        }
        ConfigCommand::Get {
            key: ConfigKey::Timezone,
        } => match current.timezone {
            Some(value) => println!("timezone: {value}"),
            None => println!("timezone: <using operating system timezone>"),
        },
        ConfigCommand::Unset {
            key: ConfigKey::Timezone,
        } => {
            current.timezone = None;
            paths.write_config(&current)?;
            println!("Unset timezone; using operating system timezone.");
        }
    }
    Ok(())
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
    let timezone = resolve_timezone(&paths, None)?;
    print_timezone_status(&paths, &timezone);
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
            print_queue_status(service.queue_locked);
        }
        Err(error) if is_service_unavailable(&error) => {
            let store = Store::open(&paths.database)?;
            let queued = store
                .list_jobs(None)?
                .into_iter()
                .filter(|job| job.state == crate::JobState::Queued)
                .count();
            println!("Scheduler: stopped");
            println!("Queued jobs: {queued}");
            print_queue_status(store.queue_locked()?);
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn print_timezone_status(paths: &StokerPaths, timezone: &ResolvedTimezone) {
    println!("Display timezone: {}", timezone.name);
    if timezone.source == TimezoneSource::Config || paths.config_path().exists() {
        println!("Timezone config: {}", paths.config_path().display());
    } else {
        println!("Timezone config: using operating system timezone");
    }
}

#[derive(Debug, Clone, Copy)]
struct QueueSnapshot {
    service_online: bool,
    locked: bool,
    queued_jobs: usize,
}

fn print_queue_status(locked: bool) {
    println!("Queue: {}", if locked { "locked" } else { "unlocked" });
    if locked {
        println!("Scheduler will not start another queued job while the queue is locked.");
    }
}

fn queue_snapshot(paths: &StokerPaths) -> anyhow::Result<QueueSnapshot> {
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(status) => Ok(QueueSnapshot {
            service_online: true,
            locked: status.queue_locked,
            queued_jobs: status.queued_jobs,
        }),
        Err(error) if is_service_unavailable(&error) => {
            let store = Store::open(&paths.database)?;
            let queued_jobs = store
                .list_jobs_with_state(None, Some(JobState::Queued))?
                .len();
            Ok(QueueSnapshot {
                service_online: false,
                locked: store.queue_locked()?,
                queued_jobs,
            })
        }
        Err(error) => Err(error),
    }
}

fn lock_queue() -> anyhow::Result<()> {
    set_queue_lock(true)
}

fn unlock_queue() -> anyhow::Result<()> {
    set_queue_lock(false)
}

fn set_queue_lock(lock: bool) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let before = queue_snapshot(&paths)?;
    let client = ServiceClient::new(paths.clone());
    let operation = if before.service_online {
        let result = if lock {
            runtime()?.block_on(client.lock_queue())
        } else {
            runtime()?.block_on(client.unlock_queue())
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if is_service_unavailable(&error) => {
                let store = Store::open(&paths.database)?;
                if lock {
                    store.lock_queue().map_err(anyhow::Error::from)
                } else {
                    store.unlock_queue().map_err(anyhow::Error::from)
                }
            }
            Err(error) => Err(error),
        }
    } else {
        let store = Store::open(&paths.database)?;
        if lock {
            store.lock_queue().map_err(anyhow::Error::from)
        } else {
            store.unlock_queue().map_err(anyhow::Error::from)
        }
    };
    operation?;

    let state = if lock {
        if before.locked {
            "Queue already locked."
        } else {
            "Queue locked."
        }
    } else if before.locked {
        "Queue unlocked."
    } else {
        "Queue already unlocked."
    };
    if lock && before.queued_jobs == 0 {
        println!("{state} No queued jobs to reorder.");
    } else {
        println!("{state}");
    }
    Ok(())
}

fn queue_edit() -> anyhow::Result<()> {
    let paths = open_paths()?;
    let snapshot = queue_snapshot(&paths)?;
    if !snapshot.locked {
        anyhow::bail!("Queue is unlocked. Run 'stoker queue lock' first.");
    }

    let store = Store::open(&paths.database)?;
    let initial_jobs = queued_jobs(&store)?;
    if initial_jobs.is_empty() {
        println!("No queued jobs to reorder.");
        return Ok(());
    }

    let client = ServiceClient::new(paths);
    queue_editor::run_queue_editor(
        initial_jobs,
        |id, target_order| {
            let result = if snapshot.service_online {
                match runtime() {
                    Ok(runtime) => match runtime.block_on(client.move_queued(id, target_order)) {
                        Ok(jobs) => Ok(jobs),
                        Err(error) if is_service_unavailable(&error) => store
                            .move_queued_job(id, target_order)
                            .map_err(anyhow::Error::from),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                }
            } else {
                store
                    .move_queued_job(id, target_order)
                    .map_err(anyhow::Error::from)
            };
            result.map_err(editor_move_error)
        },
        || queued_jobs(&store).map_err(anyhow::Error::from),
    )
}

fn queued_jobs(store: &Store) -> Result<Vec<Job>, StoreError> {
    store.list_jobs_with_state(None, Some(JobState::Queued))
}

fn editor_move_error(error: anyhow::Error) -> EditorMoveError {
    if is_stale_move_error(&error) {
        EditorMoveError::Stale
    } else {
        EditorMoveError::Callback(error)
    }
}

fn is_stale_move_error(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<StoreError>() {
        return matches!(
            error,
            StoreError::NotFound { .. }
                | StoreError::InvalidQueueOrder { .. }
                | StoreError::InvalidTransition { action: "move", .. }
        );
    }
    if error.downcast_ref::<StaleQueueMoveError>().is_some() {
        return true;
    }
    let message = error.to_string();
    message.contains("cannot move job") || message.contains("does not exist")
}

fn stop(yes: bool) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let client = ServiceClient::new(paths);
    match runtime()?.block_on(client.status()) {
        Ok(status) => {
            if let Some(id) = status.active_job
                && !yes
                && !request_confirmation(&format!(
                    "Job {id} is active. Force-cancel it and stop the scheduler"
                ))?
            {
                println!("Stop cancelled.");
                return Ok(());
            }
            runtime()?.block_on(client.stop())
        }
        Err(error) if is_service_unavailable(&error) => {
            println!("Scheduler is not running.");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn update(yes: bool) -> anyhow::Result<()> {
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
    if !yes && !request_confirmation("Continue with update")? {
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
    println!(
        "Stoker update is being finalized after this command exits. The update helper will report success or failure."
    );
    Ok(())
}

fn uninstall(yes: bool) -> anyhow::Result<()> {
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
    if !yes && !request_confirmation("Continue with uninstall")? {
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
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("schedule Windows update helper")?;
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_uninstall_script(process_id: u32, executable: &Path) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\ndel /F /Q \"{}\"\r\nstart \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1\r\nexit /B 0\r\n",
        executable.display()
    )
}

#[cfg(any(windows, test))]
fn windows_update_script(
    process_id: u32,
    executable: &Path,
    update_binary: &Path,
    update_dir: &Path,
) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\nmove /Y \"{}\" \"{}\" >NUL\r\nif errorlevel 1 (\r\n  echo Stoker update failed: could not replace the executable.\r\n  exit /b 1\r\n)\r\nrmdir /S /Q \"{}\"\r\nif errorlevel 1 (\r\n  echo Stoker update completed, but cleanup failed.\r\n) else (\r\n  echo Stoker update completed successfully.\r\n)\r\nstart \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1\r\nexit /B 0\r\n",
        update_binary.display(),
        executable.display(),
        update_dir.display()
    )
}

fn commit(id: Option<Uuid>, all: bool) -> anyhow::Result<()> {
    let paths = open_paths()?;
    if all {
        let count = runtime()?.block_on(ServiceClient::new(paths).commit_all())?;
        println!("Committed {count} DRAFT job(s).");
        return Ok(());
    }
    runtime()?.block_on(ServiceClient::new(paths).commit(id.expect("clap requires a job ID")))
}

fn pause() -> anyhow::Result<()> {
    let paths = open_paths()?;
    runtime()?.block_on(ServiceClient::new(paths).pause())?;
    println!("Paused queued jobs.");
    Ok(())
}

fn resume() -> anyhow::Result<()> {
    let paths = open_paths()?;
    runtime()?.block_on(ServiceClient::new(paths).resume())?;
    println!("Resumed paused jobs.");
    Ok(())
}

fn cancel(id: Uuid, yes: bool) -> anyhow::Result<()> {
    let paths = open_paths()?;
    if !yes && !request_confirmation(&format!("Cancel job {id}"))? {
        println!("Cancel cancelled.");
        return Ok(());
    }
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
    let command = parse_command_line(&args.command)?;

    let current_dir = std::env::current_dir().context("determine current directory")?;
    let cwd = normalize_path(
        current_dir
            .canonicalize()
            .context("resolve working directory")?,
    );
    let id = open_store()?.create_shell_job(
        NewJob {
            name: args.name,
            user: args.user,
            cwd,
            command,
        },
        args.command,
    )?;
    println!("Created job {id} (DRAFT)");
    Ok(())
}

fn parse_command_line(input: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    token.push(character);
                }
            }
            Some('"') => {
                if character == '"' {
                    quote = None;
                } else if character == '\\' && matches!(chars.peek(), Some('"') | Some('\\')) {
                    token.push(chars.next().expect("peeked character exists"));
                } else {
                    token.push(character);
                }
            }
            Some(_) => unreachable!("command parser only uses single or double quotes"),
            None if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                token_started = true;
            }
            None if character == '\\' => {
                if matches!(
                    chars.peek(),
                    Some(' ') | Some('\t') | Some('\n') | Some('\'') | Some('"') | Some('\\')
                ) {
                    token.push(chars.next().expect("peeked character exists"));
                } else {
                    token.push(character);
                }
                token_started = true;
            }
            None if character == '&' && chars.peek() == Some(&'&') => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
                chars.next();
                tokens.push("&&".to_owned());
            }
            None => {
                token.push(character);
                token_started = true;
            }
        }
    }

    if let Some(quote) = quote {
        anyhow::bail!("--cmd contains an unterminated {quote} quote");
    }
    if token_started {
        tokens.push(token);
    }
    if tokens.is_empty() {
        anyhow::bail!("--cmd must not be empty");
    }
    Ok(tokens)
}

fn show(id: Uuid, cli_timezone: Option<&str>) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let timezone = resolve_timezone(&paths, cli_timezone)?;
    let job = Store::open(&paths.database)?.get_job(id)?;
    print_job(&job, &timezone);
    Ok(())
}

fn jobs(
    owner: Option<&str>,
    state: Option<JobState>,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    let paths = open_paths()?;
    let timezone = resolve_timezone(&paths, cli_timezone)?;
    let rows: Vec<_> = Store::open(&paths.database)?
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
                timezone.format(job.created_at),
                job.committed_at
                    .map(|time| timezone.format(time))
                    .unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    let headers = [
        "queue_order",
        "job_id",
        "owner",
        "name",
        "state",
        "created_at",
        "committed_at",
    ];
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
                [
                    &row[0], &row[1], &row[2], &row[3], &row[4], &row[5], &row[6],
                ],
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

fn format_jobs_row(columns: [&str; 7], widths: &[usize; 7]) -> String {
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

fn print_job(job: &Job, timezone: &ResolvedTimezone) {
    let execution_cwd = &job.cwd;
    let execution_cwd_status = match job.state {
        JobState::Draft | JobState::Queued | JobState::Paused => "planned",
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
    println!("display_timezone: {}", timezone.name);
    println!(
        "queue_order: {}",
        job.queue_order
            .map(|order| order.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!("created_at: {}", timezone.format(job.created_at));
    println!(
        "committed_at: {}",
        format_optional_time(job.committed_at.as_ref(), timezone)
    );
    println!(
        "started_at: {}",
        format_optional_time(job.started_at.as_ref(), timezone)
    );
    println!(
        "finished_at: {}",
        format_optional_time(job.finished_at.as_ref(), timezone)
    );
    println!("exit_code: {:?}", job.exit_code);
    println!("pid: {:?}", job.pid);
    println!("failure_detail: {:?}", job.failure_detail);
}

fn format_optional_time(
    value: Option<&chrono::DateTime<chrono::Utc>>,
    timezone: &ResolvedTimezone,
) -> String {
    value
        .map(|value| timezone.format(*value))
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
    use clap::{CommandFactory, Parser};

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

    #[test]
    fn queue_operations_use_one_consistent_namespace() {
        for args in [
            ["stoker", "queue", "lock"],
            ["stoker", "queue", "edit"],
            ["stoker", "queue", "unlock"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn destructive_commands_accept_yes_flag() {
        for args in [
            vec!["stoker", "stop", "--yes"],
            vec!["stoker", "update", "--yes"],
            vec!["stoker", "uninstall", "--yes"],
            vec![
                "stoker",
                "cancel",
                "00000000-0000-0000-0000-000000000001",
                "--yes",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn timezone_aliases_are_accepted_for_all_commands() {
        assert!(Cli::try_parse_from(["stoker", "jobs", "--timezone", "Asia/Tokyo"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "stoker",
                "show",
                "00000000-0000-0000-0000-000000000001",
                "--tz",
                "UTC"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["stoker", "status", "--tz", "not/a-zone"]).is_ok());
    }

    #[test]
    fn top_level_help_describes_commands_and_timezone_option() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        for description in [
            "add        Create a DRAFT job",
            "config     Manage Stoker user configuration",
            "status     Show scheduler, queue, and timezone status",
            "queue      Lock, edit, or unlock the queue",
            "      --timezone <TIMEZONE>  Timezone used when displaying timestamps [alias: --tz]",
        ] {
            assert!(
                help.contains(description),
                "missing help text: {description}"
            );
        }
    }
}

#[cfg(test)]
mod windows_uninstall_tests {
    use super::{windows_uninstall_script, windows_update_script};
    use std::path::Path;

    #[test]
    fn uninstall_helper_waits_for_stoker_then_removes_the_binary() {
        let script = windows_uninstall_script(1234, Path::new(r"C:\Tools\stoker.exe"));
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("del /F /Q \"C:\\Tools\\stoker.exe\""));
        assert!(script.contains("start \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1"));
        assert!(script.ends_with("exit /B 0\r\n"));
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
        assert!(script.contains("Stoker update completed successfully."));
        assert!(script.contains("Stoker update failed: could not replace the executable."));
        assert!(script.contains("start \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1"));
        assert!(script.ends_with("exit /B 0\r\n"));
    }
}
