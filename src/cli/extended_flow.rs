//! Handlers for the public `stoker flow` command tree.

use anyhow::{Context, Result};
use crossterm::style::Color;
use uuid::Uuid;

use crate::config::command_cwd;
use crate::domain::flow::{
    ExecutionMode, ScheduleSpec, parse_daily, parse_every, parse_first_at, parse_once,
};
use crate::store::FlowTaskInput;
use crate::{StokerPaths, Store, output};

use super::extended_args::*;
use super::extended_flow_source;
use super::extended_output::*;
use super::extended_parsing::*;

pub(super) fn run(paths: &StokerPaths, store: &Store, command: FlowCommand) -> Result<()> {
    match command {
        FlowCommand::Create(args) => {
            let mode = store.current_mode()?;
            if mode != ExecutionMode::Scheduled {
                anyhow::bail!(
                    "flow create is only available in scheduled mode; use stoker create for serial jobs"
                );
            }
            let schedule = parse_schedule(
                paths,
                args.once_at.as_deref(),
                args.daily.as_deref(),
                args.every.as_deref(),
                args.first_at.as_deref(),
                args.schedule_timezone.as_deref(),
            )?
            .context("flow create requires --once-at, --daily, or --every")?;
            let flow = store.create_flow(args.flow_id, args.name, args.user, schedule)?;
            println!(
                "Created flow {} (DRAFT, draft revision {}).",
                flow.flow_id, flow.draft_revision
            );
        }
        FlowCommand::Commit { flow_id } => {
            let flow = store.commit_flow(&flow_id)?;
            println!(
                "Committed flow {} ({} task(s)).",
                flow.flow_id,
                flow.tasks.len()
            );
        }
        FlowCommand::Show(args) => {
            show_flow_definition(store, &args.flow_id, args.run, args.task.as_deref())?;
        }
        FlowCommand::List { user } => print_flow_list(store, user.as_deref())?,
        FlowCommand::Run(args) => {
            let request_id = args.request_id.unwrap_or_else(Uuid::new_v4);
            let run = store.create_flow_run(
                &args.flow_id,
                "MANUAL",
                args.replace_next,
                Some(request_id),
            )?;
            println!(
                "Created flow run {} for {} (request-id {}).",
                run.run_id, run.flow_id, request_id
            );
        }
        FlowCommand::Logs(args) => print_task_logs(
            paths,
            store,
            &args.flow_id,
            &args.task,
            args.run,
            args.attempt,
            args.follow,
        )?,
        FlowCommand::Cancel(args) => cancel(store, args)?,
        FlowCommand::Task(command) => task(store, command)?,
        FlowCommand::Schedule(command) => schedule(store, command)?,
        FlowCommand::Edit(command) => edit(store, command)?,
        FlowCommand::History(args) => print_runs(store, &args.flow_id)?,
        FlowCommand::Occurrences(args) => print_occurrences(store, &args.flow_id)?,
        FlowCommand::Disable(args) => set_enabled(store, &args.flow_id, false)?,
        FlowCommand::Enable(args) => set_enabled(store, &args.flow_id, true)?,
        FlowCommand::Export(args) => extended_flow_source::export(store, args)?,
        FlowCommand::SourceMode { mode } => {
            let state = store.set_flow_source_mode(mode)?;
            let colors = output::stdout_color_enabled();
            println!(
                "Flow source mode is {} (revision {}, {}).",
                output::paint_bold(state.mode, Color::Green, colors),
                output::paint(state.revision, Color::Cyan, colors),
                output::paint(state.hash, Color::Cyan, colors)
            );
            println!(
                "{}",
                output::paint("Queue remains locked.", Color::Yellow, colors)
            );
        }
        FlowCommand::Snapshot => extended_flow_source::snapshot(paths, store)?,
        FlowCommand::Sync(args) => extended_flow_source::sync(paths, store, args)?,
    }
    Ok(())
}

fn cancel(store: &Store, args: FlowCancelArgs) -> Result<()> {
    if let Some(task_id) = args.task {
        let run = store.cancel_flow_task(&args.flow_id, &task_id, args.run)?;
        println!(
            "Cancelled task {} in run {} ({:?}).",
            task_id, run.run_id, run.state
        );
    } else {
        let run = store.cancel_flow_run(&args.flow_id, args.run)?;
        println!("Cancelled flow run {} ({:?}).", run.run_id, run.state);
    }
    Ok(())
}

fn task(store: &Store, command: FlowTaskCommand) -> Result<()> {
    match command {
        FlowTaskCommand::Add(args) => {
            let definition = store.add_flow_task_with_revision(
                FlowTaskInput {
                    flow_id: args.flow_id,
                    task_id: args.task_id,
                    name: args.name,
                    cwd: command_cwd(&std::env::current_dir()?),
                    command: args.command,
                    retry: args.retries,
                    dependencies: parse_named_dependencies(args.after, args.after_failure)?,
                    depend_mode: args.match_mode,
                },
                args.revision,
            )?;
            println!(
                "Added task to flow {} (draft revision {}).",
                definition.flow_id, definition.draft_revision
            );
        }
        FlowTaskCommand::Update(args) => update_task(store, args)?,
        FlowTaskCommand::Remove(args) => {
            let flow = store.remove_flow_task(
                &args.flow_id,
                &args.task_id,
                &args.scope,
                args.run,
                args.revision,
            )?;
            println!(
                "Draft revision {} for flow {}.",
                flow.draft_revision, flow.flow_id
            );
        }
    }
    Ok(())
}

fn update_task(store: &Store, args: FlowTaskUpdateArgs) -> Result<()> {
    if args.command.is_none()
        && args.cwd.is_none()
        && args.retries.is_none()
        && args.after.is_empty()
        && args.after_failure.is_empty()
        && args.match_mode.is_none()
        && !args.clear_dependencies
    {
        anyhow::bail!("flow task update requires at least one field");
    }
    let dependencies = if args.clear_dependencies {
        Some(Vec::new())
    } else if args.after.is_empty() && args.after_failure.is_empty() {
        None
    } else {
        Some(parse_named_dependencies(args.after, args.after_failure)?)
    };
    if let Some(cwd) = &args.cwd
        && !cwd.is_dir()
    {
        anyhow::bail!("--cwd must name an existing directory");
    }
    let flow = store.set_flow_task_draft(
        &args.flow_id,
        &args.task_id,
        args.command.as_deref(),
        args.cwd.as_deref(),
        args.retries,
        dependencies,
        args.match_mode,
        args.revision,
    )?;
    println!(
        "Updated task {} in flow {} (draft revision {}).",
        args.task_id, flow.flow_id, flow.draft_revision
    );
    Ok(())
}

fn schedule(store: &Store, command: FlowScheduleCommand) -> Result<()> {
    match command {
        FlowScheduleCommand::Set(args) => set_schedule(
            store,
            &args.flow_id,
            ScheduleUpdate {
                once_at: args.once_at,
                daily: args.daily,
                every: args.every,
                first_at: args.first_at,
                timezone: args.schedule_timezone,
                revision: args.revision,
            },
        ),
    }
}

pub(super) struct ScheduleUpdate {
    pub(super) once_at: Option<String>,
    pub(super) daily: Option<String>,
    pub(super) every: Option<String>,
    pub(super) first_at: Option<String>,
    pub(super) timezone: Option<String>,
    pub(super) revision: Option<i64>,
}

pub(super) fn set_schedule(store: &Store, flow_id: &str, update: ScheduleUpdate) -> Result<()> {
    let current = store.get_flow(flow_id)?;
    let schedule = if let Some(at) = update.once_at {
        ScheduleSpec::Once {
            at: parse_once(&at).map_err(|error| anyhow::anyhow!(error))?,
        }
    } else if let Some(daily) = update.daily {
        let time = parse_daily(&daily).map_err(|error| anyhow::anyhow!(error))?;
        let timezone = update
            .timezone
            .or_else(|| {
                current
                    .schedule
                    .as_ref()
                    .and_then(|schedule| schedule.timezone().map(str::to_owned))
            })
            .context(
                "--schedule-timezone is required for a daily schedule without an existing timezone",
            )?;
        ScheduleSpec::Daily { time, timezone }
    } else if let Some(value) = update.every {
        let first_at = update
            .first_at
            .as_deref()
            .map(parse_first_at)
            .transpose()
            .map_err(|error| anyhow::anyhow!(error))?;
        ScheduleSpec::Periodic {
            every: parse_every(&value).map_err(|error| anyhow::anyhow!(error))?,
            first_at,
        }
    } else if let Some(value) = update.first_at {
        let every = match current.schedule.as_ref() {
            Some(ScheduleSpec::Periodic { every, .. }) => *every,
            _ => anyhow::bail!("--first-at can only modify an existing every schedule"),
        };
        ScheduleSpec::Periodic {
            every,
            first_at: Some(parse_first_at(&value).map_err(|error| anyhow::anyhow!(error))?),
        }
    } else if let Some(timezone) = update.timezone {
        let time = match current.schedule.as_ref() {
            Some(ScheduleSpec::Daily { time, .. }) => *time,
            _ => anyhow::bail!("--schedule-timezone can only modify an existing daily schedule"),
        };
        ScheduleSpec::Daily { time, timezone }
    } else {
        anyhow::bail!(
            "schedule set requires --once-at, --daily, --every, --first-at, or --schedule-timezone"
        );
    };
    let flow = store.set_flow_schedule_draft(flow_id, schedule, update.revision)?;
    println!(
        "Updated flow {} draft revision {}.",
        flow.flow_id, flow.draft_revision
    );
    Ok(())
}

fn edit(store: &Store, command: FlowEditCommand) -> Result<()> {
    match command {
        FlowEditCommand::Begin(args) => {
            let flow = store.freeze_flow(&args.flow_id)?;
            println!("Flow '{}' is frozen for editing.", flow.flow_id);
            println!();
            println!("Running processes continue.");
            println!("No new run, task, or retry can start until the edit is applied.");
        }
        FlowEditCommand::Apply(args) => {
            let flow = store.unfreeze_flow(&args.flow_id, args.revision)?;
            println!(
                "Applied edits to {} (graph revision {}).",
                flow.flow_id, flow.graph_revision
            );
        }
        FlowEditCommand::Discard(args) => {
            let flow = store.discard_flow_draft(&args.flow_id, args.revision)?;
            println!(
                "Discarded draft for {} (still frozen={}).",
                flow.flow_id, flow.frozen
            );
        }
    }
    Ok(())
}

pub(super) fn set_enabled(store: &Store, flow_id: &str, enabled: bool) -> Result<()> {
    let flow = store.disable_flow(flow_id, enabled)?;
    println!(
        "{} {}.",
        if enabled { "Enabled" } else { "Disabled" },
        flow.flow_id
    );
    Ok(())
}
