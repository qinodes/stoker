//! Command handlers for the scheduled and flow CLI.

use std::str::FromStr;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::domain::flow::ExecutionMode;
use crate::{StokerPaths, Store};

use super::extended_args::*;
use super::extended_flow;
use super::extended_output::*;
use super::extended_parsing::*;

pub(super) fn dispatch(paths: &StokerPaths, store: &Store, command: ExtendedCommand) -> Result<()> {
    match command {
        ExtendedCommand::Create(args) => create(paths, store, args),
        ExtendedCommand::Jobs(args) => jobs(store, args),
        ExtendedCommand::Run(args) => run_standalone(store, args),
        ExtendedCommand::Mode { command } => mode(store, command),
        ExtendedCommand::Policy { command } => policy(store, command),
        ExtendedCommand::Flow { command } => extended_flow::run(paths, store, command),
        ExtendedCommand::Show { id, run } => show_definition(store, id, run),
        ExtendedCommand::Logs(args) => logs(paths, store, args),
        ExtendedCommand::Cancel(args) => cancel_standalone_run(store, args),
        ExtendedCommand::Freeze(args) => freeze(store, &standalone_flow_id(args.id)),
        ExtendedCommand::Unfreeze(args) => unfreeze(
            store,
            &standalone_flow_id(args.id),
            args.expected_draft_revision,
        ),
        ExtendedCommand::Schedule { command } => schedule_standalone(store, command),
        ExtendedCommand::Draft { command } => draft_standalone(store, command),
        ExtendedCommand::Disable(args) => {
            extended_flow::set_enabled(store, &standalone_flow_id(args.id), false)
        }
        ExtendedCommand::Enable(args) => {
            extended_flow::set_enabled(store, &standalone_flow_id(args.id), true)
        }
        ExtendedCommand::Runs(args) => print_runs(store, &standalone_flow_id(args.id)),
        ExtendedCommand::Occurrences(args) => {
            print_occurrences(store, &standalone_flow_id(args.id))
        }
        ExtendedCommand::Recovery { command } => recovery(store, command),
        ExtendedCommand::Request { command } => request(store, command),
    }
}

fn create(paths: &StokerPaths, store: &Store, args: CreateExtendedArgs) -> Result<()> {
    if args.schedule_timezone.is_some() && args.daily.is_none() {
        anyhow::bail!("--schedule-timezone is only valid with --daily");
    }
    let schedule = parse_schedule(
        paths,
        args.once_at.as_deref(),
        args.daily.as_deref(),
        args.every.as_deref(),
        args.first_at.as_deref(),
        args.schedule_timezone.as_deref(),
    )?;
    let mode = store.current_mode()?;
    if mode == ExecutionMode::Serial && schedule.is_some() {
        anyhow::bail!(
            "serial workspace mode does not accept --once-at, --daily, --every, --first-at, or --schedule-timezone"
        );
    }
    if mode == ExecutionMode::Scheduled && schedule.is_none() {
        anyhow::bail!("scheduled workspace mode requires --once-at, --daily, or --every");
    }
    let cwd = std::env::current_dir()?.canonicalize()?;
    let job = crate::application::jobs::create_job(
        store,
        &crate::adapters::SystemWorkingDirectoryResolver,
        crate::application::CreateJobInput {
            user: args.user,
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

fn schedule_standalone(store: &Store, command: StandaloneScheduleCommand) -> Result<()> {
    match command {
        StandaloneScheduleCommand::Set(args) => extended_flow::set_schedule(
            store,
            &standalone_flow_id(args.id),
            extended_flow::ScheduleUpdate {
                once_at: args.once_at,
                daily: args.daily,
                every: args.every,
                first_at: args.first_at,
                timezone: args.schedule_timezone,
                revision: args.expected_draft_revision,
            },
        ),
    }
}

fn draft_standalone(store: &Store, command: StandaloneDraftCommand) -> Result<()> {
    match command {
        StandaloneDraftCommand::Discard(args) => {
            let revision = args
                .expected_draft_revision
                .context("--expected-draft-revision is required")?;
            let flow = store.discard_flow_draft(&standalone_flow_id(args.id), revision)?;
            println!(
                "Discarded draft for {} (still frozen={}).",
                flow.flow_id, flow.frozen
            );
        }
    }
    Ok(())
}

fn freeze(store: &Store, flow_id: &str) -> Result<()> {
    let flow = store.freeze_flow(flow_id)?;
    println!(
        "Froze {} (draft revision {}).",
        flow.flow_id, flow.draft_revision
    );
    Ok(())
}

fn unfreeze(store: &Store, flow_id: &str, revision: Option<i64>) -> Result<()> {
    let flow = store.unfreeze_flow(flow_id, revision)?;
    println!(
        "Unfroze {} (graph revision {}).",
        flow.flow_id, flow.graph_revision
    );
    Ok(())
}

fn cancel_standalone_run(store: &Store, args: StandaloneRunSelector) -> Result<()> {
    let run = store.cancel_flow_run(&standalone_flow_id(args.id), args.run)?;
    println!("Cancelled flow run {} ({:?}).", run.run_id, run.state);
    Ok(())
}

fn mode(store: &Store, command: ModeCommand) -> Result<()> {
    match command {
        ModeCommand::Show => println!("{}", store.current_mode()?),
        ModeCommand::Set { mode } => {
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

fn standalone_flow_id(id: Uuid) -> String {
    format!("standalone/{id}")
}
