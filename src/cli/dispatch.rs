//! CLI composition root and command routing.

use anyhow::Context;
use clap::Parser;
use tokio::runtime::Runtime;

use crate::adapters::LocalSchedulerGateway;
use crate::ui;
use crate::{ServiceClient, StokerPaths, Store};

use super::args::{Cli, CliCommand, DbCommand, QueueCommand, UiCommand};
use super::commands::configuration::config;
use super::commands::db::{DatabaseOperation, database};
use super::commands::jobs::{add, cancel, clean, commit, jobs, set_description, show};
use super::commands::logs::logs;
use super::commands::queue::{lock_queue, queue_edit, unlock_queue};
use super::commands::service::status;
use super::lifecycle::service::{service_run, start, stop};
use super::update::{uninstall, update};

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run_command_with_timezone(cli.command, cli.timezone)
}

pub fn run_command(command: CliCommand) -> anyhow::Result<()> {
    run_command_with_timezone(command, None)
}

pub(crate) fn run_command_with_timezone(
    command: CliCommand,
    timezone: Option<String>,
) -> anyhow::Result<()> {
    let paths = open_paths()?;
    run_command_with_paths(command, timezone.as_deref(), &paths)
}

pub(crate) fn run_command_with_paths(
    command: CliCommand,
    timezone: Option<&str>,
    paths: &StokerPaths,
) -> anyhow::Result<()> {
    match command {
        CliCommand::Add(args) => add(paths, args),
        CliCommand::SetDescription(args) => set_description(paths, args),
        CliCommand::Show { id } => show(paths, id, timezone),
        CliCommand::Jobs { user, state } => jobs(paths, user.as_deref(), state, timezone),
        CliCommand::Config { command } => config(paths, command, timezone),
        CliCommand::Db { command } => match command {
            DbCommand::Check { integrity } => {
                database(paths, DatabaseOperation::Check { integrity })
            }
            DbCommand::Backup { destination } => {
                database(paths, DatabaseOperation::Backup { destination })
            }
            DbCommand::Restore { source, yes } => {
                database(paths, DatabaseOperation::Restore { source, yes })
            }
        },
        CliCommand::Clean => clean(paths),
        CliCommand::Update(args) => update(args.yes),
        CliCommand::Uninstall(args) => uninstall(args.yes),
        CliCommand::Start => start(paths),
        CliCommand::ServiceRun => service_run(paths),
        CliCommand::Status => status(paths),
        CliCommand::Queue { command } => match command {
            QueueCommand::Lock => lock_queue(paths),
            QueueCommand::Edit => queue_edit(paths),
            QueueCommand::Unlock => unlock_queue(paths),
        },
        CliCommand::Stop(args) => stop(paths, args.yes),
        CliCommand::Ui { command } => match command {
            UiCommand::Start(args) => ui::start(paths.clone(), args.host, args.port, args.open),
            UiCommand::Status => ui::status(paths.clone()),
            UiCommand::Stop => ui::stop(paths.clone()),
        },
        CliCommand::UiRun { host, port } => ui::run(paths.clone(), host, port),
        CliCommand::Commit { ids, all, user } => commit(paths, ids, all, user),
        CliCommand::Cancel(args) => cancel(paths, args.id, args.confirmation.yes),
        CliCommand::Logs { id, follow } => logs(paths, id, follow),
    }
}

pub(crate) fn open_store(paths: &StokerPaths) -> anyhow::Result<Store> {
    Store::open(&paths.database).context("open Stoker database")
}

fn open_paths() -> anyhow::Result<StokerPaths> {
    let paths = StokerPaths::from_env()?;
    paths.ensure()?;
    Ok(paths)
}

pub(crate) fn scheduler_gateway(paths: &StokerPaths) -> LocalSchedulerGateway {
    ServiceClient::new(paths.clone())
}

pub(crate) fn runtime() -> anyhow::Result<Runtime> {
    Runtime::new().context("create scheduler runtime")
}
