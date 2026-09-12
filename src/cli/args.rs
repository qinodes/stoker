//! Declarative CLI schema. This module performs no I/O.

use std::net::IpAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::domain::JobState;

#[derive(Debug, Parser)]
#[command(
    name = "stoker",
    about = "Jobs run from the directory where you submit them.",
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
    #[command(about = "Set or clear a job's description")]
    SetDescription(SetDescriptionArgs),
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
    #[command(about = "Manage scheduler execution and log policies")]
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    #[command(about = "Check, back up, or restore the SQLite database")]
    Db {
        #[command(subcommand)]
        command: DbCommand,
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
    #[command(about = "Start, stop, or inspect the browser UI")]
    Ui {
        #[command(subcommand)]
        command: UiCommand,
    },
    #[command(name = "ui-run", hide = true)]
    UiRun {
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        #[arg(long, default_value_t = crate::ui::default_port())]
        port: u16,
    },
    #[command(about = "Commit a DRAFT job to the queue")]
    Commit {
        #[arg(
            value_name = "JOB_ID",
            num_args = 1..,
            required_unless_present_any = ["all", "user"],
            conflicts_with_all = ["all", "user"],
            help = "One or more Job IDs"
        )]
        ids: Vec<Uuid>,
        #[arg(long, help = "Commit all DRAFT jobs in creation order")]
        all: bool,
        #[arg(
            long,
            value_name = "NAME",
            conflicts_with_all = ["ids", "all"],
            help = "Commit all DRAFT jobs for this user in creation order"
        )]
        user: Option<String>,
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

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    #[command(about = "Check SQLite health")]
    Check {
        #[arg(long, help = "Run SQLite integrity_check instead of quick_check")]
        integrity: bool,
    },
    #[command(about = "Create a consistent SQLite backup")]
    Backup {
        #[arg(value_name = "PATH", help = "Optional destination path")]
        destination: Option<PathBuf>,
    },
    #[command(about = "Restore SQLite from a backup")]
    Restore {
        #[arg(value_name = "PATH", help = "Backup database to restore")]
        source: PathBuf,
        #[arg(long, help = "Confirm replacing the current database")]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigKey {
    Timezone,
}

#[derive(Debug, Subcommand)]
pub enum PolicyCommand {
    #[command(about = "Set a policy value")]
    Set {
        #[arg(value_enum)]
        key: PolicyKey,
        #[arg(
            help = "Policy value (whole MB for log capacities, a count for retained jobs, or milliseconds for runtime limits)"
        )]
        value: Option<String>,
    },
    #[command(about = "Show the current scheduler policies")]
    Show,
    #[command(about = "Get a policy value")]
    Get {
        #[arg(value_enum)]
        key: PolicyKey,
    },
    #[command(about = "Reset a policy value to its built-in default")]
    Unset {
        #[arg(value_enum)]
        key: PolicyKey,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PolicyKey {
    LogMaxBytesPerJob,
    LogSegmentBytes,
    LogMaxBytesTotal,
    LogRetentionJobs,
    LogDiskReserveBytes,
    TerminationGraceMs,
    MaxRuntimeMs,
    StartupTimeoutMs,
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

#[derive(Debug, Subcommand)]
pub enum UiCommand {
    #[command(about = "Start the browser UI server")]
    Start(UiStartArgs),
    #[command(about = "Show browser UI server status")]
    Status,
    #[command(about = "Stop the browser UI server")]
    Stop,
}

#[derive(Debug, Args)]
pub struct UiStartArgs {
    #[arg(
        long,
        default_value = "127.0.0.1",
        help = "Address to bind; non-loopback addresses expose the UI to the network"
    )]
    pub host: IpAddr,
    #[arg(long, default_value_t = crate::ui::default_port(), help = "TCP port for the browser UI")]
    pub port: u16,
    #[arg(long, help = "Open the UI in the default browser after startup")]
    pub open: bool,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long, help = "Logical job owner label")]
    pub user: String,
    #[arg(long, help = "Job name")]
    pub name: String,
    #[arg(long, help = "Optional job description (maximum 200 characters)")]
    pub description: Option<String>,
    #[arg(
        long = "cmd",
        required = true,
        allow_hyphen_values = true,
        help = "Complete shell command string (quote it when it contains spaces)"
    )]
    pub command: String,
}

#[derive(Debug, Args)]
pub struct SetDescriptionArgs {
    #[arg(help = "Job ID")]
    pub id: Uuid,
    #[arg(
        value_name = "DESCRIPTION",
        required_unless_present = "clear",
        conflicts_with = "clear",
        help = "New job description (maximum 200 characters)"
    )]
    pub description: Option<String>,
    #[arg(long, help = "Clear the current description")]
    pub clear: bool,
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

pub(crate) fn parse_job_state(value: &str) -> Result<JobState, String> {
    value.to_ascii_uppercase().parse()
}
