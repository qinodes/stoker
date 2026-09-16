//! Command handlers for the extended Task 003 CLI.

use std::str::FromStr;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::domain::flow::{ExecutionMode, ScheduleSpec, parse_daily, parse_once};
use crate::store::FlowTaskInput;
use crate::{StokerPaths, Store};

use super::extended_args::*;
use super::extended_output::*;
use super::extended_parsing::*;

pub(super) fn dispatch(paths: &StokerPaths, store: &Store, command: ExtendedCommand) -> Result<()> {
    match command {
        ExtendedCommand::Add(args) => add(paths, store, args),
        ExtendedCommand::Jobs(args) => jobs(store, args),
        ExtendedCommand::Run(args) => run_standalone(store, args),
        ExtendedCommand::Mode { command } => mode(store, command),
        ExtendedCommand::Policy { command } => policy(store, command),
        ExtendedCommand::Flow { command } => flow(paths, store, command),
        ExtendedCommand::Show { id, run } => show_definition(store, &id, run),
        ExtendedCommand::Logs(args) => logs(paths, store, args),
        ExtendedCommand::Cancel(args) => cancel_run(store, args),
        ExtendedCommand::Freeze(args) => freeze(store, &args.id),
        ExtendedCommand::Unfreeze(args) => unfreeze(store, &args.id, args.expected_draft_revision),
        ExtendedCommand::Schedule { command } => schedule(store, command),
        ExtendedCommand::Draft { command } => draft(store, command),
        ExtendedCommand::Disable(args) => disable(store, &args.id, false),
        ExtendedCommand::Enable(args) => disable(store, &args.id, true),
        ExtendedCommand::Runs(args) => print_runs(store, &args.id),
        ExtendedCommand::Occurrences(args) => print_occurrences(store, &args.id),
        ExtendedCommand::Recovery { command } => recovery(store, command),
        ExtendedCommand::Request { command } => request(store, command),
    }
}

fn add(paths: &StokerPaths, store: &Store, args: AddExtendedArgs) -> Result<()> {
    let depend_mode = parse_depend_mode(&args.depend_mode)?;
    let dependencies = parse_dependencies(
        args.depend_on,
        args.depend_status,
        args.dependencies,
        args.depend_mode,
    )?;
    if let Some(flow_id) = args.flow_id {
        let task_id = args
            .task_id
            .context("--task-id is required when --flow-id is used")?;
        if let Some(user) = &args.user {
            let flow = store.get_flow(&flow_id)?;
            if flow.owner != *user {
                anyhow::bail!("task owner must match flow owner {}", flow.owner);
            }
        }
        let cwd = portable_command_cwd(&std::env::current_dir()?);
        let definition = store.add_flow_task_with_revision(
            FlowTaskInput {
                flow_id,
                task_id,
                name: args.name,
                cwd,
                command: args.command,
                retry: args.retry,
                dependencies,
                depend_mode,
            },
            args.expected_draft_revision,
        )?;
        println!(
            "Added task to flow {} (draft revision {}).",
            definition.flow_id, definition.draft_revision
        );
        return Ok(());
    }
    if !dependencies.is_empty() {
        anyhow::bail!("dependencies require --flow-id");
    }
    if args.schedule_timezone.is_some() && args.daily.is_none() {
        anyhow::bail!("--schedule-timezone is only valid with --daily");
    }
    let schedule = parse_schedule(
        paths,
        args.at.as_deref(),
        args.daily.as_deref(),
        args.schedule_timezone.as_deref(),
    )?;
    let mode = store.current_mode()?;
    if mode == ExecutionMode::Serial && schedule.is_some() {
        anyhow::bail!(
            "serial workspace mode does not accept --at, --daily, or --schedule-timezone"
        );
    }
    if mode == ExecutionMode::Scheduled && schedule.is_none() {
        anyhow::bail!("scheduled workspace mode requires --at or --daily");
    }
    let cwd = std::env::current_dir()?.canonicalize()?;
    let job = crate::application::jobs::create_job(
        store,
        &crate::adapters::SystemWorkingDirectoryResolver,
        crate::application::CreateJobInput {
            user: args
                .user
                .context("--user is required for a standalone job")?,
            name: args.name,
            description: args.description,
            cwd,
            command_line: args.command,
        },
    )?;
    store.configure_standalone(job.id, mode, schedule, args.retry)?;
    println!("Created job {} ({mode}, DRAFT).", job.id);
    Ok(())
}

fn flow(paths: &StokerPaths, store: &Store, command: FlowCommand) -> Result<()> {
    match command {
        FlowCommand::Add(args) => {
            let mode = store.current_mode()?;
            if mode != ExecutionMode::Scheduled {
                anyhow::bail!(
                    "flow add is only available in scheduled mode; use stoker add for serial jobs"
                );
            }
            let schedule = parse_schedule(
                paths,
                args.at.as_deref(),
                args.daily.as_deref(),
                args.schedule_timezone.as_deref(),
            )?
            .context("flow add requires --at or --daily")?;
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
        FlowCommand::Show { flow_id, run } => {
            show_definition(store, &flow_id, run)?;
        }
        FlowCommand::List { user } => {
            for flow in store.list_flows(user.as_deref())? {
                print_flow(&flow);
            }
        }
        FlowCommand::Run(args) => {
            let request_id = args.request_id.unwrap_or_else(Uuid::new_v4);
            let run =
                store.create_flow_run(&args.flow_id, "MANUAL", args.skip_next, Some(request_id))?;
            println!(
                "Created flow run {} for {} (request-id {}).",
                run.run_id, run.flow_id, request_id
            );
        }
        FlowCommand::Freeze(args) => freeze(store, &args.id)?,
        FlowCommand::Unfreeze(args) => unfreeze(store, &args.id, args.expected_draft_revision)?,
        FlowCommand::Task(command) => flow_task(paths, store, command)?,
        FlowCommand::Schedule(command) => schedule(store, command)?,
        FlowCommand::Draft(command) => draft(store, command)?,
        FlowCommand::Runs(args) => print_runs(store, &args.id)?,
        FlowCommand::Occurrences(args) => print_occurrences(store, &args.id)?,
        FlowCommand::Cancel(args) => {
            let run = store.cancel_flow_run(&args.flow_id, args.run)?;
            println!("Cancelled flow run {} ({:?}).", run.run_id, run.state);
        }
        FlowCommand::Disable(args) => disable(store, &args.id, false)?,
        FlowCommand::Enable(args) => disable(store, &args.id, true)?,
    }
    Ok(())
}

fn flow_task(paths: &StokerPaths, store: &Store, command: FlowTaskCommand) -> Result<()> {
    match command {
        FlowTaskCommand::Remove(args) => {
            let flow = store.remove_flow_task(
                &args.flow_id,
                &args.task_id,
                &args.scope,
                args.run,
                args.expected_draft_revision,
            )?;
            println!(
                "Draft revision {} for flow {}.",
                flow.draft_revision, flow.flow_id
            );
        }
        FlowTaskCommand::Cancel(args) => {
            let run = store.cancel_flow_task(&args.flow_id, &args.task_id, args.run)?;
            println!(
                "Cancelled task {} in run {} ({:?}).",
                args.task_id, run.run_id, run.state
            );
        }
        FlowTaskCommand::Show(args) => {
            let run = store.get_flow_run(args.run)?;
            if run.flow_id != args.flow_id {
                anyhow::bail!("run does not belong to flow");
            }
            let task = run
                .tasks
                .iter()
                .find(|task| task.task_id == args.task_id)
                .context("task does not exist in run")?;
            println!(
                "run={} task_id={} state={:?} attempts={}",
                run.run_id, task.task_id, task.state, task.attempt_count
            );
        }
        FlowTaskCommand::Logs(args) => print_task_logs(
            paths,
            store,
            &args.flow_id,
            &args.task_id,
            args.run,
            args.attempt,
            args.follow,
        )?,
        FlowTaskCommand::Set(args) => {
            if args.command.is_none()
                && args.cwd.is_none()
                && args.retry.is_none()
                && args.dependencies.is_empty()
                && args.depend_mode.is_none()
                && !args.clear_dependencies
            {
                anyhow::bail!("flow task set requires at least one field");
            }
            let dependencies = if args.clear_dependencies {
                Some(Vec::new())
            } else if args.dependencies.is_empty() {
                None
            } else {
                Some(parse_dependency_values(args.dependencies)?)
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
                args.retry,
                dependencies,
                args.depend_mode,
                args.expected_draft_revision,
            )?;
            println!(
                "Updated task {} in flow {} (draft revision {}).",
                args.task_id, flow.flow_id, flow.draft_revision
            );
        }
    }
    Ok(())
}

fn schedule(store: &Store, command: ScheduleCommand) -> Result<()> {
    match command {
        ScheduleCommand::Set(args) => {
            let flow_id = resolve_definition_id(store, &args.id)?;
            let current = store.get_flow(&flow_id)?;
            let schedule = if let Some(at) = args.at {
                ScheduleSpec::Once {
                    at: parse_once(&at).map_err(|error| anyhow::anyhow!(error))?,
                }
            } else if let Some(daily) = args.daily {
                let time = parse_daily(&daily).map_err(|error| anyhow::anyhow!(error))?;
                let timezone = args.schedule_timezone.or_else(|| current.schedule.as_ref().and_then(|schedule| schedule.timezone().map(str::to_owned))).context("--schedule-timezone is required for a daily schedule without an existing timezone")?;
                ScheduleSpec::Daily { time, timezone }
            } else if let Some(timezone) = args.schedule_timezone {
                let time = match current.schedule.as_ref() {
                    Some(ScheduleSpec::Daily { time, .. }) => *time,
                    _ => anyhow::bail!(
                        "--schedule-timezone can only modify an existing daily schedule"
                    ),
                };
                ScheduleSpec::Daily { time, timezone }
            } else {
                anyhow::bail!("schedule set requires --at or --daily");
            };
            let flow =
                store.set_flow_schedule_draft(&flow_id, schedule, args.expected_draft_revision)?;
            println!(
                "Updated flow {} draft revision {}.",
                flow.flow_id, flow.draft_revision
            );
        }
    }
    Ok(())
}

fn draft(store: &Store, command: DraftCommand) -> Result<()> {
    match command {
        DraftCommand::Discard(args) => {
            let revision = args
                .expected_draft_revision
                .context("--expected-draft-revision is required")?;
            let flow_id = resolve_definition_id(store, &args.id)?;
            let flow = store.discard_flow_draft(&flow_id, revision)?;
            println!(
                "Discarded draft for {} (still frozen={}).",
                flow.flow_id, flow.frozen
            );
        }
    }
    Ok(())
}
fn freeze(store: &Store, id: &str) -> Result<()> {
    let flow_id = resolve_definition_id(store, id)?;
    let flow = store.freeze_flow(&flow_id)?;
    println!(
        "Froze {} (draft revision {}).",
        flow.flow_id, flow.draft_revision
    );
    Ok(())
}
fn unfreeze(store: &Store, id: &str, revision: Option<i64>) -> Result<()> {
    let flow_id = resolve_definition_id(store, id)?;
    let flow = store.unfreeze_flow(&flow_id, revision)?;
    println!(
        "Unfroze {} (graph revision {}).",
        flow.flow_id, flow.graph_revision
    );
    Ok(())
}
fn disable(store: &Store, id: &str, enabled: bool) -> Result<()> {
    let flow_id = resolve_definition_id(store, id)?;
    let flow = store.disable_flow(&flow_id, enabled)?;
    println!(
        "{} {}.",
        if enabled { "Enabled" } else { "Disabled" },
        flow.flow_id
    );
    Ok(())
}

fn cancel_run(store: &Store, args: FlowRunSelector) -> Result<()> {
    let flow_id = resolve_definition_id(store, &args.flow_id)?;
    let run = store.cancel_flow_run(&flow_id, args.run)?;
    println!("Cancelled flow run {} ({:?}).", run.run_id, run.state);
    Ok(())
}

fn mode(store: &Store, command: ModeCommand) -> Result<()> {
    match command {
        ModeCommand::Show => println!("{}", store.current_mode()?),
        ModeCommand::Set { mode } => {
            store.lock_queue()?;
            store.set_mode(mode)?;
            println!("Mode set to {mode}; queue remains locked.");
        }
    }
    Ok(())
}
fn policy(store: &Store, command: ExtendedPolicyCommand) -> Result<()> {
    match command {
        ExtendedPolicyCommand::Show => {
            println!("max-concurrency={}", store.scheduled_concurrency()?)
        }
        ExtendedPolicyCommand::Set { key, value } if key == "max-concurrency" => {
            println!(
                "max-concurrency={}",
                store.set_scheduled_concurrency(
                    value.context("max-concurrency requires a positive integer")?
                )?
            );
        }
        ExtendedPolicyCommand::Unset { key } if key == "max-concurrency" => {
            println!("max-concurrency={}", store.set_scheduled_concurrency(2)?)
        }
        _ => anyhow::bail!("unknown extended policy; use max-concurrency"),
    }
    Ok(())
}
fn run_standalone(store: &Store, args: RunArgs) -> Result<()> {
    let definition = store.standalone_definition(args.id)?;
    if definition.mode != ExecutionMode::Scheduled {
        anyhow::bail!("job {} is not scheduled", args.id);
    }
    let request_id = args.request_id.unwrap_or_else(Uuid::new_v4);
    let run = store.create_standalone_run(args.id, args.skip_next, Some(request_id))?;
    println!(
        "Created manual run {} for job {} (request-id {}).",
        run.run_id, args.id, request_id
    );
    Ok(())
}
fn jobs(store: &Store, args: ExtendedJobsArgs) -> Result<()> {
    let mode = args
        .mode
        .as_deref()
        .map(ExecutionMode::from_str)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    let state = args
        .state
        .as_deref()
        .map(crate::domain::JobState::from_str)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    for job in store.list_jobs_for_mode(args.user.as_deref(), state, mode)? {
        println!("{} {} {} {}", job.id, job.user, job.name, job.state);
    }
    Ok(())
}
fn recovery(store: &Store, command: RecoveryCommand) -> Result<()> {
    match command {
        RecoveryCommand::Reconcile {
            execution_id,
            confirm_stopped,
        } => {
            let id =
                Uuid::parse_str(&execution_id).context("execution-id must be a flow run UUID")?;
            store.reconcile_recovery(id, confirm_stopped)?;
            println!(
                "Reconciled recovery for {execution_id}; queue may be unlocked after all recoveries are resolved."
            );
        }
    }
    Ok(())
}
fn request(store: &Store, command: RequestCommand) -> Result<()> {
    match command {
        RequestCommand::Show { request_id } => {
            let request = store.manual_request(request_id)?;
            println!(
                "request_id={} flow_id={} run_id={} result={}",
                request.request_id,
                request.flow_id,
                request
                    .run_id
                    .map_or_else(|| "".into(), |id| id.to_string()),
                request.result
            );
        }
    }
    Ok(())
}

fn portable_command_cwd(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    let text = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| text.into_owned());
    #[cfg(not(windows))]
    let text = text.into_owned();
    text.replace('\\', "/")
}
