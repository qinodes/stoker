//! Clap data types for the scheduled and flow command surface.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand};
use uuid::Uuid;

use crate::domain::flow::{DependencyMode, ExecutionMode};

pub(super) use super::extended_standalone_args::*;

#[derive(Debug, Parser)]
#[command(name = "stoker", disable_help_subcommand = true)]
pub(super) struct ExtendedCli {
    #[arg(long = "timezone", global = true)]
    pub(super) timezone: Option<String>,
    #[command(subcommand)]
    pub(super) command: ExtendedCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum ExtendedCommand {
    #[command(about = "Create a DRAFT standalone job")]
    Create(CreateExtendedArgs),
    #[command(about = "List standalone jobs")]
    Jobs(ExtendedJobsArgs),
    Run(RunArgs),
    Mode {
        #[command(subcommand)]
        command: ModeCommand,
    },
    Policy {
        #[command(subcommand)]
        command: ExtendedPolicyCommand,
    },
    Flow {
        #[command(subcommand)]
        command: FlowCommand,
    },
    Show {
        id: Uuid,
        #[arg(long)]
        run: Option<Uuid>,
    },
    Logs(LogsArgs),
    Cancel(StandaloneRunSelector),
    Freeze(StandaloneDefinitionIdArgs),
    Unfreeze(StandaloneUnfreezeArgs),
    Schedule {
        #[command(subcommand)]
        command: StandaloneScheduleCommand,
    },
    Draft {
        #[command(subcommand)]
        command: StandaloneDraftCommand,
    },
    Disable(StandaloneDefinitionIdArgs),
    Enable(StandaloneDefinitionIdArgs),
    Runs(StandaloneDefinitionIdArgs),
    Occurrences(StandaloneDefinitionIdArgs),
    Recovery {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    Request {
        #[command(subcommand)]
        command: RequestCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum ModeCommand {
    Show,
    Set { mode: ExecutionMode },
}

#[derive(Debug, Subcommand)]
pub(super) enum ExtendedPolicyCommand {
    Set { key: String, value: Option<u32> },
    Unset { key: String },
    Show,
}

#[derive(Debug, Subcommand)]
pub(super) enum FlowCommand {
    #[command(about = "Create a draft flow definition")]
    Create(FlowCreateArgs),
    #[command(about = "Validate and commit a draft flow")]
    Commit { flow_id: String },
    #[command(about = "Show a flow definition or one of its runs")]
    Show(FlowShowArgs),
    #[command(about = "List flows without expanding their tasks")]
    List {
        #[arg(long)]
        user: Option<String>,
    },
    #[command(about = "Start a manual flow run")]
    Run(FlowRunArgs),
    #[command(about = "Read logs for a task in a flow run")]
    Logs(FlowLogsArgs),
    #[command(about = "Cancel a flow run or one task within it")]
    Cancel(FlowCancelArgs),
    #[command(about = "Add, update, or remove flow tasks")]
    #[command(subcommand)]
    Task(FlowTaskCommand),
    #[command(about = "Edit a flow's future schedule")]
    #[command(subcommand)]
    Schedule(FlowScheduleCommand),
    #[command(about = "Begin, apply, or discard a safe flow edit")]
    #[command(subcommand)]
    Edit(FlowEditCommand),
    #[command(about = "List run history for a flow")]
    History(FlowDefinitionIdArgs),
    #[command(about = "List scheduled occurrences for a flow")]
    Occurrences(FlowDefinitionIdArgs),
    #[command(about = "Disable future automatic triggers")]
    Disable(FlowDefinitionIdArgs),
    #[command(about = "Enable future automatic triggers")]
    Enable(FlowDefinitionIdArgs),
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("flow_schedule")
        .required(true)
        .multiple(false)
        .args(["once_at", "daily", "every"])
))]
pub(super) struct FlowCreateArgs {
    #[arg(value_name = "FLOW_ID", help = "Stable ID for the new flow")]
    pub(super) flow_id: String,
    #[arg(long, help = "Logical flow owner")]
    pub(super) user: String,
    #[arg(long, help = "Display name")]
    pub(super) name: String,
    #[arg(
        long = "once-at",
        value_name = "RFC3339",
        help = "Run once at an RFC 3339 instant with a UTC offset"
    )]
    pub(super) once_at: Option<String>,
    #[arg(
        long = "daily",
        value_name = "HH:mm",
        help = "Run daily at this local wall-clock time"
    )]
    pub(super) daily: Option<String>,
    #[arg(
        long = "every",
        value_name = "Nm|Nh",
        help = "Run at a fixed elapsed interval, such as 15m or 2h"
    )]
    pub(super) every: Option<String>,
    #[arg(
        long = "first-at",
        value_name = "RFC3339",
        help = "Set the first periodic occurrence",
        requires = "every"
    )]
    pub(super) first_at: Option<String>,
    #[arg(
        long = "schedule-timezone",
        value_name = "IANA_ZONE",
        help = "Use this IANA timezone for a daily schedule",
        requires = "daily"
    )]
    pub(super) schedule_timezone: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct FlowDefinitionIdArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
}

#[derive(Debug, Args)]
pub(super) struct FlowShowArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(long, value_name = "RUN_ID", help = "Show a specific run")]
    pub(super) run: Option<Uuid>,
    #[arg(
        long,
        value_name = "TASK_ID",
        requires = "run",
        help = "Filter the run to one task"
    )]
    pub(super) task: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct FlowRunArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(long, help = "Replace the next scheduled occurrence after startup")]
    pub(super) replace_next: bool,
    #[arg(long)]
    pub(super) request_id: Option<Uuid>,
}

#[derive(Debug, Args)]
pub(super) struct FlowLogsArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(long, value_name = "RUN_ID")]
    pub(super) run: Uuid,
    #[arg(long, value_name = "TASK_ID")]
    pub(super) task: String,
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) attempt: Option<u32>,
    #[arg(short = 'f', long)]
    pub(super) follow: bool,
}

#[derive(Debug, Args)]
pub(super) struct FlowCancelArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(long, value_name = "RUN_ID")]
    pub(super) run: Uuid,
    #[arg(long, value_name = "TASK_ID", help = "Cancel only this task")]
    pub(super) task: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(super) enum FlowTaskCommand {
    #[command(about = "Add a task to a flow draft")]
    Add(FlowTaskAddArgs),
    #[command(about = "Update a task in a frozen flow's draft")]
    Update(FlowTaskUpdateArgs),
    #[command(about = "Remove a task from future or current execution")]
    Remove(FlowTaskRemoveArgs),
}

#[derive(Debug, Args)]
pub(super) struct FlowTaskAddArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(value_name = "TASK_ID")]
    pub(super) task_id: String,
    #[arg(long)]
    pub(super) name: String,
    #[arg(
        long = "cmd",
        alias = "command",
        required = true,
        allow_hyphen_values = true
    )]
    pub(super) command: String,
    #[arg(
        long = "after",
        value_name = "TASK_ID",
        help = "Require successful completion; repeatable"
    )]
    pub(super) after: Vec<String>,
    #[arg(
        long = "after-failure",
        value_name = "TASK_ID",
        help = "Require failed completion; repeatable"
    )]
    pub(super) after_failure: Vec<String>,
    #[arg(long = "match", value_name = "all|any", default_value = "all")]
    pub(super) match_mode: DependencyMode,
    #[arg(long = "retries", value_name = "N", default_value_t = 0)]
    pub(super) retries: u32,
    #[arg(long = "revision", value_name = "N", help = "Expected draft revision")]
    pub(super) revision: Option<i64>,
}

#[derive(Debug, Args)]
pub(super) struct FlowTaskRemoveArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(value_name = "TASK_ID")]
    pub(super) task_id: String,
    #[arg(long, default_value = "future")]
    pub(super) scope: String,
    #[arg(long)]
    pub(super) run: Option<Uuid>,
    #[arg(long = "revision")]
    pub(super) revision: Option<i64>,
}

#[derive(Debug, Args)]
pub(super) struct FlowTaskUpdateArgs {
    #[arg(value_name = "FLOW_ID")]
    pub(super) flow_id: String,
    #[arg(value_name = "TASK_ID")]
    pub(super) task_id: String,
    #[arg(long = "cmd", alias = "command")]
    pub(super) command: Option<String>,
    #[arg(long)]
    pub(super) cwd: Option<PathBuf>,
    #[arg(long = "retries")]
    pub(super) retries: Option<u32>,
    #[arg(long = "after", conflicts_with = "clear_dependencies")]
    pub(super) after: Vec<String>,
    #[arg(long = "after-failure", conflicts_with = "clear_dependencies")]
    pub(super) after_failure: Vec<String>,
    #[arg(long = "match")]
    pub(super) match_mode: Option<DependencyMode>,
    #[arg(long, conflicts_with_all = ["after", "after_failure"])]
    pub(super) clear_dependencies: bool,
    #[arg(long = "revision")]
    pub(super) revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub(super) enum FlowScheduleCommand {
    Set(FlowScheduleSetArgs),
}

#[derive(Debug, Args)]
pub(super) struct FlowScheduleSetArgs {
    pub(super) flow_id: String,
    #[arg(
        long = "once-at",
        value_name = "RFC3339",
        help = "Set a future one-time occurrence",
        conflicts_with_all = ["daily", "every", "first_at", "schedule_timezone"]
    )]
    pub(super) once_at: Option<String>,
    #[arg(
        long,
        value_name = "HH:mm",
        help = "Set the daily local wall-clock time",
        conflicts_with_all = ["once_at", "every", "first_at"]
    )]
    pub(super) daily: Option<String>,
    #[arg(
        long,
        value_name = "Nm|Nh",
        help = "Set a fixed elapsed interval, such as 15m or 2h",
        conflicts_with_all = ["once_at", "daily", "schedule_timezone"]
    )]
    pub(super) every: Option<String>,
    #[arg(
        long = "first-at",
        value_name = "RFC3339",
        help = "Set the first occurrence of an existing periodic schedule",
        conflicts_with_all = ["once_at", "daily", "schedule_timezone"]
    )]
    pub(super) first_at: Option<String>,
    #[arg(
        long = "schedule-timezone",
        value_name = "IANA_ZONE",
        help = "Set the timezone of an existing daily schedule"
    )]
    pub(super) schedule_timezone: Option<String>,
    #[arg(long = "revision")]
    pub(super) revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub(super) enum FlowEditCommand {
    #[command(about = "Freeze a flow and begin editing its draft")]
    Begin(FlowDefinitionIdArgs),
    #[command(about = "Validate, apply, and unfreeze a flow")]
    Apply(FlowEditApplyArgs),
    #[command(about = "Discard the draft but keep the flow frozen")]
    Discard(FlowEditDiscardArgs),
}

#[derive(Debug, Args)]
pub(super) struct FlowEditApplyArgs {
    pub(super) flow_id: String,
    #[arg(long = "revision")]
    pub(super) revision: Option<i64>,
}

#[derive(Debug, Args)]
pub(super) struct FlowEditDiscardArgs {
    pub(super) flow_id: String,
    #[arg(long = "revision")]
    pub(super) revision: i64,
}

#[derive(Debug, Subcommand)]
pub(super) enum RecoveryCommand {
    Reconcile {
        execution_id: String,
        #[arg(long)]
        confirm_stopped: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum RequestCommand {
    Show { request_id: Uuid },
}
