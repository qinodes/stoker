use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::config::{
    ConfigSnapshot, ConfigSnapshotEntry, ConfigSnapshotFile, ConfigSnapshotReason,
    ResolvedTimezone, StokerConfig, TimezoneSource, normalize_path, resolve_timezone,
};
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
        #[arg(help = "IANA timezone identifier; omit it to choose interactively")]
        value: Option<String>,
    },
    #[command(about = "Show the current configuration")]
    Show,
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
    #[command(about = "Interactively restore a previous configuration snapshot")]
    Restore,
    #[command(about = "Create a manual configuration snapshot")]
    Snapshot,
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
        CliCommand::Config { command } => config(command, timezone.as_deref()),
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

fn config(command: ConfigCommand, cli_timezone: Option<&str>) -> anyhow::Result<()> {
    let paths = open_paths()?;
    match command {
        ConfigCommand::Set {
            key: ConfigKey::Timezone,
            value,
        } => {
            let mut current = paths.read_config()?;
            let value = match value {
                Some(value) => value,
                None => {
                    let Some(value) = select_timezone(current.timezone.as_deref())? else {
                        return Ok(());
                    };
                    value
                }
            };
            resolve_timezone(&paths, Some(&value))?;
            current.timezone = Some(value.clone());
            paths.write_config(&current)?;
            println!("Set timezone to {value}.");
        }
        ConfigCommand::Show => {
            let current = paths.read_config()?;
            println!("Stoker configuration");
            println!("File: {}", paths.config_path().display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&current).context("format Stoker configuration")?
            );
        }
        ConfigCommand::Get {
            key: ConfigKey::Timezone,
        } => match paths.read_config()?.timezone {
            Some(value) => println!("timezone: {value}"),
            None => println!("timezone: <using operating system timezone>"),
        },
        ConfigCommand::Unset {
            key: ConfigKey::Timezone,
        } => {
            let mut current = paths.read_config()?;
            current.timezone = None;
            paths.write_config(&current)?;
            println!("Unset timezone; using operating system timezone.");
        }
        ConfigCommand::Restore => restore_config(&paths, cli_timezone)?,
        ConfigCommand::Snapshot => {
            let current = paths.read_config()?;
            let path = paths.create_config_snapshot(&current, ConfigSnapshotReason::Manual)?;
            println!("Created configuration snapshot: {}.", path.display());
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct TimezoneSelectorState {
    timezones: Vec<String>,
    query: String,
    selected: usize,
}

impl TimezoneSelectorState {
    fn new(mut timezones: Vec<String>, current: Option<&str>) -> Self {
        timezones.sort();
        timezones.dedup();
        let selected = current
            .and_then(|value| timezones.iter().position(|timezone| timezone == value))
            .unwrap_or(0);
        Self {
            timezones,
            query: String::new(),
            selected,
        }
    }

    fn matches(&self) -> Vec<usize> {
        let query = self.query.to_ascii_lowercase();
        self.timezones
            .iter()
            .enumerate()
            .filter_map(|(index, timezone)| {
                if timezone.to_ascii_lowercase().contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }

    fn selected_timezone(&self) -> Option<&str> {
        self.matches()
            .get(self.selected)
            .and_then(|index| self.timezones.get(*index))
            .map(String::as_str)
    }

    fn reduce(&mut self, key: KeyEvent) -> TimezoneSelectorAction {
        if key.code == KeyCode::Char('c')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return TimezoneSelectorAction::Exit;
        }

        let matches = self.matches();
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                TimezoneSelectorAction::None
            }
            KeyCode::Down => {
                if !matches.is_empty() {
                    self.selected = (self.selected + 1).min(matches.len() - 1);
                }
                TimezoneSelectorAction::None
            }
            KeyCode::Enter => self
                .selected_timezone()
                .map(|timezone| TimezoneSelectorAction::Select(timezone.to_owned()))
                .unwrap_or(TimezoneSelectorAction::None),
            KeyCode::Esc => TimezoneSelectorAction::Exit,
            KeyCode::Char('q') => TimezoneSelectorAction::Exit,
            KeyCode::Char(character) => {
                self.query.push(character);
                self.selected = 0;
                TimezoneSelectorAction::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                TimezoneSelectorAction::None
            }
            _ => TimezoneSelectorAction::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TimezoneSelectorAction {
    None,
    Exit,
    Select(String),
}

fn select_timezone(current: Option<&str>) -> anyhow::Result<Option<String>> {
    let timezones = chrono_tz::TZ_VARIANTS
        .iter()
        .map(ToString::to_string)
        .collect();
    terminal::enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = InteractiveTerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide).context("enter timezone selector")?;
    let mut state = TimezoneSelectorState::new(timezones, current);

    loop {
        render_timezone_selector(&mut stdout, &state)?;
        let key = loop {
            if let Event::Key(key) = event::read().context("read timezone selector input")?
                && key.kind != KeyEventKind::Release
            {
                break key;
            }
        };
        match state.reduce(key) {
            TimezoneSelectorAction::None => {}
            TimezoneSelectorAction::Exit => return Ok(None),
            TimezoneSelectorAction::Select(timezone) => return Ok(Some(timezone)),
        }
    }
}

fn render_timezone_selector(
    stdout: &mut impl Write,
    state: &TimezoneSelectorState,
) -> anyhow::Result<()> {
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0)).context("clear timezone selector")?;
    let matches = state.matches();
    let mut output = String::new();
    output.push_str("Select timezone\n");
    output.push_str(&format!(
        "Search: {}  ({} matches)\n",
        state.query,
        matches.len()
    ));
    output.push_str(
        "↑/↓ select || type to search || Backspace delete || Enter save || q/Esc cancel\n\n",
    );

    if matches.is_empty() {
        output.push_str("No matching IANA timezones.\n");
    } else {
        const VISIBLE_ROWS: usize = 12;
        let start = state
            .selected
            .saturating_sub(VISIBLE_ROWS / 2)
            .min(matches.len().saturating_sub(VISIBLE_ROWS));
        let end = (start + VISIBLE_ROWS).min(matches.len());
        for (visible_index, match_index) in matches[start..end].iter().enumerate() {
            let index = start + visible_index;
            let marker = if index == state.selected { '>' } else { ' ' };
            output.push_str(&format!("{marker} {}\n", state.timezones[*match_index]));
        }
    }

    stdout
        .write_all(output.as_bytes())
        .context("write timezone selector")?;
    stdout.flush().context("flush timezone selector")?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotView {
    List,
    Detail,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotSelectorState {
    selected: usize,
    view: SnapshotView,
}

impl SnapshotSelectorState {
    fn new() -> Self {
        Self {
            selected: 0,
            view: SnapshotView::List,
        }
    }

    fn reduce(&mut self, key: KeyEvent, entries: &[ConfigSnapshotEntry]) -> SnapshotSelectorAction {
        if key.code == KeyCode::Char('c')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return SnapshotSelectorAction::Exit;
        }

        match self.view {
            SnapshotView::List => match key.code {
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    SnapshotSelectorAction::None
                }
                KeyCode::Down => {
                    if !entries.is_empty() {
                        self.selected = (self.selected + 1).min(entries.len() - 1);
                    }
                    SnapshotSelectorAction::None
                }
                KeyCode::Enter if !entries.is_empty() => {
                    self.view = SnapshotView::Detail;
                    SnapshotSelectorAction::None
                }
                KeyCode::Char('q') | KeyCode::Esc => SnapshotSelectorAction::Exit,
                _ => SnapshotSelectorAction::None,
            },
            SnapshotView::Detail => match key.code {
                KeyCode::Enter
                    if matches!(
                        entries.get(self.selected),
                        Some(ConfigSnapshotEntry::Valid(_))
                    ) =>
                {
                    self.view = SnapshotView::Confirm;
                    SnapshotSelectorAction::None
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    self.view = SnapshotView::List;
                    SnapshotSelectorAction::None
                }
                _ => SnapshotSelectorAction::None,
            },
            SnapshotView::Confirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    SnapshotSelectorAction::Restore(self.selected)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.view = SnapshotView::List;
                    SnapshotSelectorAction::None
                }
                _ => SnapshotSelectorAction::None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotSelectorAction {
    None,
    Exit,
    Restore(usize),
}

struct InteractiveTerminalGuard;

impl Drop for InteractiveTerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn restore_config(paths: &StokerPaths, cli_timezone: Option<&str>) -> anyhow::Result<()> {
    let entries = paths.list_config_snapshots()?;
    if entries.is_empty() {
        println!(
            "No configuration snapshots found in {}.",
            paths.snapshot_dir().display()
        );
        return Ok(());
    }

    let timezone = resolve_timezone(paths, cli_timezone)?;
    let current = paths.read_config()?;
    let Some(selected) = select_config_snapshot(&entries, &current, &timezone)? else {
        return Ok(());
    };

    let changed = paths.restore_config_snapshot(&selected.snapshot)?;
    if changed {
        println!("Restored configuration from {}.", selected.path.display());
    } else {
        println!("Configuration already matches that snapshot.");
    }
    Ok(())
}

fn select_config_snapshot(
    entries: &[ConfigSnapshotEntry],
    current: &StokerConfig,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<Option<ConfigSnapshotFile>> {
    terminal::enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = InteractiveTerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide).context("enter snapshot selector")?;
    let mut state = SnapshotSelectorState::new();

    loop {
        render_snapshot_selector(&mut stdout, &state, entries, current, timezone)?;
        let key = loop {
            if let Event::Key(key) = event::read().context("read snapshot selector input")?
                && key.kind != KeyEventKind::Release
            {
                break key;
            }
        };
        match state.reduce(key, entries) {
            SnapshotSelectorAction::None => {}
            SnapshotSelectorAction::Exit => return Ok(None),
            SnapshotSelectorAction::Restore(index) => {
                let Some(ConfigSnapshotEntry::Valid(snapshot)) = entries.get(index) else {
                    continue;
                };
                return Ok(Some(snapshot.clone()));
            }
        }
    }
}

fn render_snapshot_selector(
    stdout: &mut impl Write,
    state: &SnapshotSelectorState,
    entries: &[ConfigSnapshotEntry],
    current: &StokerConfig,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<()> {
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0)).context("clear snapshot selector")?;
    let mut output = String::new();
    match state.view {
        SnapshotView::List => {
            output.push_str("Stoker configuration snapshots\n");
            output.push_str("↑/↓ select || Enter details || q/Esc quit\n\n");
            let reason_width = snapshot_reason_width(entries);
            output.push_str(&snapshot_list_header(reason_width));
            for (index, entry) in entries.iter().enumerate() {
                let marker = if index == state.selected { '>' } else { ' ' };
                match entry {
                    ConfigSnapshotEntry::Valid(snapshot) => {
                        output.push_str(&format_snapshot_list_row(
                            marker,
                            timezone.format(snapshot.snapshot.created_at),
                            snapshot.snapshot.reason,
                            snapshot_summary(current, &snapshot.snapshot),
                            reason_width,
                        ));
                    }
                    ConfigSnapshotEntry::Invalid { path, .. } => {
                        output.push_str(&format_snapshot_list_row(
                            marker,
                            "-",
                            "invalid snapshot",
                            format!(
                                "unavailable: {}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("<unknown file>")
                            ),
                            reason_width,
                        ));
                    }
                }
            }
        }
        SnapshotView::Detail => {
            let entry = &entries[state.selected];
            output.push_str("Snapshot details (read-only)\n");
            output.push_str("q/Esc back to snapshot list\n\n");
            match entry {
                ConfigSnapshotEntry::Valid(snapshot) => {
                    output.push_str(&format!("File: {}\n", snapshot.path.display()));
                    output.push_str("Changes from current configuration:\n");
                    output.push_str(&format!(
                        "{}\n\n",
                        detailed_snapshot_summary(current, &snapshot.snapshot)
                    ));
                    output.push_str("Snapshot JSON:\n");
                    output.push_str(&snapshot_json_for_display(&snapshot.snapshot, timezone)?);
                    output.push_str("\n\nEnter restore this snapshot || q/Esc back");
                }
                ConfigSnapshotEntry::Invalid { path, error } => {
                    output.push_str(&format!("File: {}\n\n", path.display()));
                    output.push_str("This snapshot is unavailable and cannot be restored.\n");
                    output.push_str(&format!("Reason: {error}\n\nq/Esc back"));
                }
            }
        }
        SnapshotView::Confirm => {
            let snapshot = match &entries[state.selected] {
                ConfigSnapshotEntry::Valid(snapshot) => snapshot,
                ConfigSnapshotEntry::Invalid { .. } => {
                    return Ok(());
                }
            };
            output.push_str("Restore configuration snapshot?\n\n");
            output.push_str(&format!("File: {}\n", snapshot.path.display()));
            output.push_str(&format!(
                "Created time: {}\n",
                timezone.format(snapshot.snapshot.created_at)
            ));
            output.push_str("\nThe current configuration will be saved as a new snapshot first.\n");
            output.push_str("Continue? [y/N, q/Esc cancel]");
        }
    }
    stdout
        .write_all(output.as_bytes())
        .context("write snapshot selector")?;
    stdout.flush().context("flush snapshot selector")?;
    Ok(())
}

fn snapshot_reason_width(entries: &[ConfigSnapshotEntry]) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            ConfigSnapshotEntry::Valid(snapshot) => snapshot.snapshot.reason.to_string().len(),
            ConfigSnapshotEntry::Invalid { .. } => "invalid snapshot".len(),
        })
        .max()
        .unwrap_or(0)
        .max("Reason".len())
}

fn snapshot_list_header(reason_width: usize) -> String {
    format!(
        "  {:<29}  {:<reason_width$} {}\n",
        "Created time",
        "Reason",
        "Summary",
        reason_width = reason_width
    )
}

fn format_snapshot_list_row(
    marker: char,
    created_time: impl std::fmt::Display,
    reason: impl std::fmt::Display,
    summary: impl std::fmt::Display,
    reason_width: usize,
) -> String {
    let created_time = created_time.to_string();
    let reason = reason.to_string();
    let summary = summary.to_string();
    format!(
        "{marker} {created_time:<29}  {reason:<reason_width$} {summary}\n",
        reason_width = reason_width
    )
}

fn snapshot_summary(current: &StokerConfig, snapshot: &ConfigSnapshot) -> String {
    let keys = changed_config_keys(current, &snapshot.config);
    if keys.is_empty() {
        "same as current".to_owned()
    } else {
        format!("{} changed: {}", keys.len(), summarize_keys(&keys))
    }
}

fn detailed_snapshot_summary(current: &StokerConfig, snapshot: &ConfigSnapshot) -> String {
    let keys = changed_config_keys(current, &snapshot.config);
    if keys.is_empty() {
        "No differences.".to_owned()
    } else {
        keys.into_iter()
            .map(|key| format!("  {key}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn snapshot_json_for_display(
    snapshot: &ConfigSnapshot,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<String> {
    let created_at = snapshot.created_at;
    let mut snapshot_json =
        serde_json::to_value(snapshot).context("format snapshot configuration")?;
    if let Some(object) = snapshot_json.as_object_mut() {
        object.insert(
            "created_at".to_owned(),
            serde_json::Value::String(timezone.format(created_at)),
        );
    }
    serde_json::to_string_pretty(&snapshot_json).context("format snapshot configuration")
}

fn changed_config_keys(current: &StokerConfig, snapshot: &StokerConfig) -> Vec<String> {
    let current = serde_json::to_value(current).expect("StokerConfig serializes");
    let snapshot = serde_json::to_value(snapshot).expect("StokerConfig serializes");
    let (Some(current), Some(snapshot)) = (current.as_object(), snapshot.as_object()) else {
        return Vec::new();
    };
    let mut keys = current
        .keys()
        .chain(snapshot.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    keys.retain(|key| current.get(key) != snapshot.get(key));
    keys.into_iter().collect()
}

fn summarize_keys(keys: &[String]) -> String {
    let value = keys.join(", ");
    if value.chars().count() <= 32 {
        value
    } else {
        let prefix = value.chars().take(29).collect::<String>();
        format!("{prefix}...")
    }
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
    fn pause_and_resume_commands_are_removed() {
        assert!(Cli::try_parse_from(["stoker", "pause"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "resume"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "queue", "pause"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "queue", "resume"]).is_err());
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
    fn config_restore_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "restore"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "config", "restore", "--tz", "UTC"]).is_ok());
    }

    #[test]
    fn config_snapshot_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "snapshot"]).is_ok());
    }

    #[test]
    fn config_show_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "show"]).is_ok());
    }

    #[test]
    fn timezone_set_accepts_an_omitted_or_explicit_value() {
        assert!(Cli::try_parse_from(["stoker", "config", "set", "timezone"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "config", "set", "timezone", "Asia/Tokyo"]).is_ok());
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
mod snapshot_selector_tests {
    use super::*;
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn valid_entry() -> ConfigSnapshotEntry {
        ConfigSnapshotEntry::Valid(ConfigSnapshotFile {
            path: PathBuf::from("config-test.json"),
            snapshot: ConfigSnapshot {
                snapshot_version: 1,
                created_at: Utc::now(),
                reason: crate::config::ConfigSnapshotReason::Initial,
                config: StokerConfig {
                    timezone: Some("Asia/Tokyo".to_owned()),
                },
            },
        })
    }

    #[test]
    fn enter_opens_details_and_escape_returns_to_list() {
        let entries = vec![valid_entry()];
        let mut state = SnapshotSelectorState::new();

        assert_eq!(
            state.reduce(key(KeyCode::Enter), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::Detail);
        assert_eq!(
            state.reduce(key(KeyCode::Esc), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::List);
    }

    #[test]
    fn enter_then_yes_requests_restore_and_no_returns_to_list() {
        let entries = vec![valid_entry()];
        let mut state = SnapshotSelectorState::new();
        state.reduce(key(KeyCode::Enter), &entries);
        state.reduce(key(KeyCode::Enter), &entries);

        assert_eq!(state.view, SnapshotView::Confirm);
        assert_eq!(
            state.reduce(key(KeyCode::Char('n')), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::List);

        state.reduce(key(KeyCode::Enter), &entries);
        state.reduce(key(KeyCode::Enter), &entries);
        assert_eq!(
            state.reduce(key(KeyCode::Char('y')), &entries),
            SnapshotSelectorAction::Restore(0)
        );
    }

    #[test]
    fn invalid_snapshot_cannot_enter_confirmation() {
        let entries = vec![ConfigSnapshotEntry::Invalid {
            path: PathBuf::from("broken.json"),
            error: "invalid JSON".to_owned(),
        }];
        let mut state = SnapshotSelectorState::new();
        state.reduce(key(KeyCode::Enter), &entries);

        assert_eq!(state.view, SnapshotView::Detail);
        assert_eq!(
            state.reduce(key(KeyCode::Enter), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::Detail);
    }

    #[test]
    fn snapshot_list_header_columns_align_with_rows() {
        let reason_width = "manual".len();
        let header = snapshot_list_header(reason_width);
        let row = format_snapshot_list_row(
            '>',
            "2026-09-05T14:13:52.329+09:00",
            crate::config::ConfigSnapshotReason::Manual,
            "same as current",
            reason_width,
        );

        assert_eq!(header.find("Reason"), row.find("manual"));
        assert_eq!(header.find("Summary"), row.find("same as current"));
    }

    #[test]
    fn snapshot_json_includes_metadata_and_localizes_created_at() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        paths.ensure().expect("initialize temporary paths");

        let snapshot = match valid_entry() {
            ConfigSnapshotEntry::Valid(snapshot) => snapshot.snapshot,
            ConfigSnapshotEntry::Invalid { .. } => unreachable!(),
        };
        let timezone = resolve_timezone(&paths, Some("Asia/Tokyo")).expect("valid timezone");

        let json = snapshot_json_for_display(&snapshot, &timezone).expect("valid snapshot JSON");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert!(value.get("snapshot_version").is_some());
        assert_eq!(value["reason"], "initial");
        assert_eq!(value["config"]["timezone"], "Asia/Tokyo");
        assert!(value["created_at"].as_str().unwrap().ends_with("+09:00"));
    }

    #[test]
    fn timezone_selector_searches_and_selects_a_timezone() {
        let mut state = TimezoneSelectorState::new(
            vec![
                "UTC".to_owned(),
                "Asia/Taipei".to_owned(),
                "Asia/Tokyo".to_owned(),
            ],
            Some("Asia/Taipei"),
        );

        assert_eq!(state.selected_timezone(), Some("Asia/Taipei"));
        state.reduce(key(KeyCode::Char('t')));
        state.reduce(key(KeyCode::Char('o')));
        assert_eq!(state.selected_timezone(), Some("Asia/Tokyo"));
        assert_eq!(
            state.reduce(key(KeyCode::Enter)),
            TimezoneSelectorAction::Select("Asia/Tokyo".to_owned())
        );
    }

    #[test]
    fn timezone_selector_can_cancel_and_clear_search() {
        let mut state = TimezoneSelectorState::new(
            vec!["Asia/Taipei".to_owned(), "Asia/Tokyo".to_owned()],
            None,
        );

        state.reduce(key(KeyCode::Char('t')));
        state.reduce(key(KeyCode::Backspace));
        assert_eq!(state.query, "");
        assert_eq!(
            state.reduce(key(KeyCode::Char('q'))),
            TimezoneSelectorAction::Exit
        );

        let mut state = TimezoneSelectorState::new(
            vec!["Asia/Taipei".to_owned(), "Asia/Tokyo".to_owned()],
            None,
        );
        assert_eq!(
            state.reduce(key(KeyCode::Esc)),
            TimezoneSelectorAction::Exit
        );
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
