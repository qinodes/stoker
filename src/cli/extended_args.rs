//! Clap data types for the extended Task 003 command surface.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand};
use uuid::Uuid;

use crate::domain::flow::{DependencyMode, ExecutionMode};

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
    Add(AddExtendedArgs),
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
        id: String,
        #[arg(long)]
        run: Option<Uuid>,
    },
    Logs(LogsArgs),
    Cancel(FlowRunSelector),
    Freeze(DefinitionIdArgs),
    Unfreeze(UnfreezeArgs),
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommand,
    },
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
    },
    Disable(DefinitionIdArgs),
    Enable(DefinitionIdArgs),
    Runs(DefinitionIdArgs),
    Occurrences(DefinitionIdArgs),
    Recovery {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    Request {
        #[command(subcommand)]
        command: RequestCommand,
    },
}

#[derive(Debug, Args)]
pub(super) struct AddExtendedArgs {
    #[arg(long)]
    pub(super) user: Option<String>,
    #[arg(long)]
    pub(super) name: String,
    #[arg(long)]
    pub(super) description: Option<String>,
    #[arg(
        long = "cmd",
        alias = "command",
        required = true,
        allow_hyphen_values = true
    )]
    pub(super) command: String,
    #[arg(long = "flow-id", conflicts_with_all = ["at", "daily", "schedule_timezone"])]
    pub(super) flow_id: Option<String>,
    #[arg(long = "task-id", requires = "flow_id")]
    pub(super) task_id: Option<String>,
    #[arg(long = "at", conflicts_with = "daily")]
    pub(super) at: Option<String>,
    #[arg(long = "daily", conflicts_with = "at")]
    pub(super) daily: Option<String>,
    #[arg(long = "schedule-timezone", requires = "daily")]
    pub(super) schedule_timezone: Option<String>,
    #[arg(long, default_value_t = 0)]
    pub(super) retry: u32,
    #[arg(long = "depend-on", requires = "depend_status")]
    pub(super) depend_on: Option<String>,
    #[arg(long = "depend-status", requires = "depend_on")]
    pub(super) depend_status: Option<String>,
    #[arg(long = "dependency")]
    pub(super) dependencies: Vec<String>,
    #[arg(long = "depend-mode", default_value = "all")]
    pub(super) depend_mode: String,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}

#[derive(Debug, Args)]
pub(super) struct ExtendedJobsArgs {
    #[arg(long)]
    pub(super) user: Option<String>,
    #[arg(long)]
    pub(super) state: Option<String>,
    #[arg(long)]
    pub(super) mode: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct RunArgs {
    pub(super) id: Uuid,
    #[arg(long)]
    pub(super) skip_next: bool,
    #[arg(long)]
    pub(super) request_id: Option<Uuid>,
}
#[derive(Debug, Args)]
pub(super) struct DefinitionIdArgs {
    pub(super) id: String,
}
#[derive(Debug, Args)]
pub(super) struct LogsArgs {
    pub(super) id: String,
    #[arg(long)]
    pub(super) run: Uuid,
    #[arg(long)]
    pub(super) attempt: Option<u32>,
    #[arg(short = 'f', long)]
    pub(super) follow: bool,
}
#[derive(Debug, Args)]
pub(super) struct UnfreezeArgs {
    pub(super) id: String,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
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
    Add(FlowAddArgs),
    Commit {
        flow_id: String,
    },
    Show {
        flow_id: String,
        #[arg(long)]
        run: Option<Uuid>,
    },
    List {
        #[arg(long)]
        user: Option<String>,
    },
    Run(FlowRunArgs),
    Freeze(DefinitionIdArgs),
    Unfreeze(UnfreezeArgs),
    #[command(subcommand)]
    Task(FlowTaskCommand),
    #[command(subcommand)]
    Schedule(ScheduleCommand),
    #[command(subcommand)]
    Draft(DraftCommand),
    Runs(DefinitionIdArgs),
    Occurrences(DefinitionIdArgs),
    Cancel(FlowRunSelector),
    Disable(DefinitionIdArgs),
    Enable(DefinitionIdArgs),
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("flow_schedule")
        .required(true)
        .multiple(false)
        .args(["at", "daily"])
))]
pub(super) struct FlowAddArgs {
    #[arg(long)]
    pub(super) user: String,
    #[arg(long)]
    pub(super) name: String,
    #[arg(long = "flow-id")]
    pub(super) flow_id: String,
    #[arg(long = "at")]
    pub(super) at: Option<String>,
    #[arg(long = "daily")]
    pub(super) daily: Option<String>,
    #[arg(long = "schedule-timezone", requires = "daily")]
    pub(super) schedule_timezone: Option<String>,
}
#[derive(Debug, Args)]
pub(super) struct FlowRunArgs {
    pub(super) flow_id: String,
    #[arg(long)]
    pub(super) skip_next: bool,
    #[arg(long)]
    pub(super) request_id: Option<Uuid>,
}

#[derive(Debug, Subcommand)]
pub(super) enum FlowTaskCommand {
    Remove(FlowTaskSelector),
    Set(FlowTaskSetArgs),
    Show(FlowTaskRunSelector),
    Logs(FlowTaskLogsSelector),
    Cancel(FlowTaskRunSelector),
}
#[derive(Debug, Args)]
pub(super) struct FlowTaskSelector {
    pub(super) flow_id: String,
    pub(super) task_id: String,
    #[arg(long, default_value = "future")]
    pub(super) scope: String,
    #[arg(long)]
    pub(super) run: Option<Uuid>,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}
#[derive(Debug, Args)]
pub(super) struct FlowTaskRunSelector {
    pub(super) flow_id: String,
    pub(super) task_id: String,
    #[arg(long)]
    pub(super) run: Uuid,
}
#[derive(Debug, Args)]
pub(super) struct FlowTaskLogsSelector {
    pub(super) flow_id: String,
    pub(super) task_id: String,
    #[arg(long)]
    pub(super) run: Uuid,
    #[arg(long)]
    pub(super) attempt: Option<u32>,
    #[arg(short = 'f', long)]
    pub(super) follow: bool,
}
#[derive(Debug, Args)]
pub(super) struct FlowTaskSetArgs {
    pub(super) flow_id: String,
    pub(super) task_id: String,
    #[arg(long = "cmd", alias = "command")]
    pub(super) command: Option<String>,
    #[arg(long)]
    pub(super) cwd: Option<PathBuf>,
    #[arg(long)]
    pub(super) retry: Option<u32>,
    #[arg(long = "dependency", conflicts_with = "clear_dependencies")]
    pub(super) dependencies: Vec<String>,
    #[arg(long)]
    pub(super) depend_mode: Option<DependencyMode>,
    #[arg(long, conflicts_with = "dependencies")]
    pub(super) clear_dependencies: bool,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub(super) enum ScheduleCommand {
    Set(ScheduleSetArgs),
}
#[derive(Debug, Args)]
pub(super) struct ScheduleSetArgs {
    pub(super) id: String,
    #[arg(long, conflicts_with = "daily")]
    pub(super) at: Option<String>,
    #[arg(long, conflicts_with = "at")]
    pub(super) daily: Option<String>,
    #[arg(long = "schedule-timezone")]
    pub(super) schedule_timezone: Option<String>,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}
#[derive(Debug, Subcommand)]
pub(super) enum DraftCommand {
    Discard(UnfreezeArgs),
}
#[derive(Debug, Args)]
pub(super) struct FlowRunSelector {
    pub(super) flow_id: String,
    #[arg(long)]
    pub(super) run: Uuid,
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
