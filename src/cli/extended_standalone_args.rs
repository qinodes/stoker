//! Clap data types owned by the standalone scheduled-job command surface.

use clap::{Args, Subcommand};
use uuid::Uuid;

#[derive(Debug, Args)]
pub(super) struct CreateExtendedArgs {
    #[arg(long)]
    pub(super) user: String,
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
    #[arg(
        long = "once-at",
        value_name = "RFC3339",
        help = "Run once at an RFC 3339 instant with a UTC offset",
        conflicts_with_all = ["daily", "every", "first_at"]
    )]
    pub(super) once_at: Option<String>,
    #[arg(
        long = "daily",
        value_name = "HH:mm",
        help = "Run daily at this local wall-clock time",
        conflicts_with_all = ["once_at", "every", "first_at"]
    )]
    pub(super) daily: Option<String>,
    #[arg(
        long = "every",
        value_name = "Nm|Nh",
        help = "Run at a fixed elapsed interval, such as 15m or 2h",
        conflicts_with_all = ["once_at", "daily"]
    )]
    pub(super) every: Option<String>,
    #[arg(
        long = "first-at",
        value_name = "RFC3339",
        help = "Set the first periodic occurrence",
        requires = "every",
        conflicts_with_all = ["once_at", "daily"]
    )]
    pub(super) first_at: Option<String>,
    #[arg(
        long = "schedule-timezone",
        value_name = "IANA_ZONE",
        help = "Use this IANA timezone for a daily schedule",
        requires = "daily"
    )]
    pub(super) schedule_timezone: Option<String>,
    #[arg(long, default_value_t = 0)]
    pub(super) retry: u32,
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
pub(super) struct StandaloneDefinitionIdArgs {
    pub(super) id: Uuid,
}

#[derive(Debug, Args)]
pub(super) struct LogsArgs {
    pub(super) id: Uuid,
    #[arg(long)]
    pub(super) run: Uuid,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub(super) attempt: Option<u32>,
    #[arg(short = 'f', long)]
    pub(super) follow: bool,
}

#[derive(Debug, Args)]
pub(super) struct StandaloneUnfreezeArgs {
    pub(super) id: Uuid,
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub(super) enum StandaloneScheduleCommand {
    Set(StandaloneScheduleSetArgs),
}

#[derive(Debug, Args)]
pub(super) struct StandaloneScheduleSetArgs {
    pub(super) id: Uuid,
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
    #[arg(long)]
    pub(super) expected_draft_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub(super) enum StandaloneDraftCommand {
    Discard(StandaloneUnfreezeArgs),
}

#[derive(Debug, Args)]
pub(super) struct StandaloneRunSelector {
    pub(super) id: Uuid,
    #[arg(long)]
    pub(super) run: Uuid,
}
