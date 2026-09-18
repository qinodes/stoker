//! Job command orchestration.

use anyhow::Context;
use crossterm::style::Color;
use uuid::Uuid;

use crate::adapters::SystemWorkingDirectoryResolver;
use crate::application::{self, CommitSelection, CreateJobInput, DescriptionUpdate, JobFilter};
use crate::config::{normalize_path, resolve_timezone};
use crate::domain::JobState;
use crate::output;
use crate::{StokerPaths, Store};

use super::super::presentation::jobs::{format_jobs_row_for_terminal, print_job};
use super::super::{
    AddArgs, SetDescriptionArgs, application_cli_error, open_store, print_success, print_warning,
    request_confirmation, runtime, scheduler_gateway,
};

pub(crate) fn commit(
    paths: &StokerPaths,
    ids: Vec<Uuid>,
    all: bool,
    user: Option<String>,
) -> anyhow::Result<()> {
    let scheduler = scheduler_gateway(paths);
    let selection = if all {
        CommitSelection::All
    } else if let Some(user) = user.clone() {
        CommitSelection::User(user)
    } else {
        CommitSelection::Jobs(ids.clone())
    };
    let committed = runtime()?
        .block_on(application::jobs::commit_jobs(&scheduler, selection))
        .map_err(application_cli_error)?;
    let count = committed.len();
    if all {
        print_success(format!("Committed {count} DRAFT job(s)."));
        return Ok(());
    }

    if let Some(user) = user {
        if count == 0 {
            print_warning(format!("No DRAFT jobs found for user '{user}'."));
        } else {
            print_success(format!("Committed {count} DRAFT job(s) for user '{user}'."));
        }
        return Ok(());
    }

    if ids.len() > 1 {
        print_success(format!("Committed {count} DRAFT job(s)."));
        return Ok(());
    }

    let id = ids.into_iter().next().expect("clap requires a job ID");
    print_success(format!("Committed job {id} (QUEUED)."));
    Ok(())
}

pub(crate) fn cancel(paths: &StokerPaths, id: Uuid, yes: bool) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    if let Ok(definition) = store.standalone_definition(id)
        && definition.mode == crate::domain::flow::ExecutionMode::Scheduled
    {
        anyhow::bail!("scheduled standalone cancel requires --run <RUN_ID>");
    }
    if !yes && !request_confirmation(&format!("Cancel job {id}"))? {
        print_warning("Cancel cancelled.");
        return Ok(());
    }
    let scheduler = scheduler_gateway(paths);
    runtime()?
        .block_on(application::jobs::cancel_job(&scheduler, id))
        .map_err(application_cli_error)?;
    print_success(format!("Cancelled job {id}."));
    Ok(())
}

pub(crate) fn create(paths: &StokerPaths, args: AddArgs) -> anyhow::Result<()> {
    if args.user.trim().is_empty() {
        anyhow::bail!("--user must not be empty");
    }
    if args.name.trim().is_empty() {
        anyhow::bail!("--name must not be empty");
    }
    let current_dir = std::env::current_dir().context("determine current directory")?;
    let cwd = normalize_path(
        current_dir
            .canonicalize()
            .context("resolve working directory")?,
    );
    let store = open_store(paths)?;
    let job = application::jobs::create_job(
        &store,
        &SystemWorkingDirectoryResolver,
        CreateJobInput {
            user: args.user,
            name: args.name,
            cwd,
            command_line: args.command,
            description: args.description,
        },
    )
    .map_err(application_cli_error)?;
    let id = job.id;
    let cwd = job.cwd;
    print_success(format!("Created job {id} (DRAFT)"));
    println!("Working directory: {}", cwd.display());
    println!();
    println!(
        "Next: {}",
        output::paint_bold(
            format!("stoker show {id}"),
            Color::Cyan,
            output::stdout_color_enabled(),
        )
    );
    println!(
        "      {}",
        output::paint_bold(
            format!("stoker commit {id}"),
            Color::Cyan,
            output::stdout_color_enabled(),
        )
    );
    Ok(())
}

pub(crate) fn set_description(paths: &StokerPaths, args: SetDescriptionArgs) -> anyhow::Result<()> {
    let store = open_store(paths)?;
    let current = application::jobs::job_detail(&store, args.id).map_err(application_cli_error)?;
    let description = if args.clear { None } else { args.description };
    let updated = application::jobs::update_description(
        &store,
        DescriptionUpdate {
            id: args.id,
            description,
            expected_revision: current.description_revision,
        },
    )
    .map_err(application_cli_error)?;
    if updated.description.is_some() {
        print_success(format!("Updated description for job {}.", updated.id));
    } else {
        print_success(format!("Cleared description for job {}.", updated.id));
    }
    Ok(())
}

pub(crate) fn show(
    paths: &StokerPaths,
    id: Uuid,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    let timezone = resolve_timezone(paths, cli_timezone)?;
    let store = Store::open(&paths.database)?;
    let job = application::jobs::job_detail(&store, id).map_err(application_cli_error)?;
    print_job(&job, &timezone);
    if let Ok(definition) = store.standalone_definition(id) {
        println!("mode={}", definition.mode);
        if let Some(schedule) = definition.schedule {
            println!("schedule={schedule:?}");
        }
        println!(
            "retry={} enabled={} generation={}",
            definition.retry, definition.enabled, definition.generation
        );
    }
    Ok(())
}

pub(crate) fn jobs(
    paths: &StokerPaths,
    owner: Option<&str>,
    state: Option<JobState>,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    let timezone = resolve_timezone(paths, cli_timezone)?;
    let store = Store::open(&paths.database)?;
    let rows: Vec<_> = application::jobs::query_jobs(
        &store,
        &JobFilter {
            user: owner.map(str::to_owned),
            state,
        },
    )
    .map_err(application_cli_error)?
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
    let colors_enabled = output::stdout_color_enabled();
    println!(
        "{}",
        format_jobs_row_for_terminal(headers, &widths, true, colors_enabled)
    );
    for row in &rows {
        println!(
            "{}",
            format_jobs_row_for_terminal(
                [
                    &row[0], &row[1], &row[2], &row[3], &row[4], &row[5], &row[6],
                ],
                &widths,
                false,
                colors_enabled,
            )
        );
    }
    Ok(())
}

pub(crate) fn clean(paths: &StokerPaths) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let jobs = application::jobs::clean_jobs(&store, paths).map_err(application_cli_error)?;
    print_success(format!("Cleaned {} terminal job(s).", jobs.len()));
    Ok(())
}
