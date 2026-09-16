//! Queue command orchestration and interactive editor adaptation.

use crossterm::style::Color;

use crate::application::{self, ApplicationError, JobFilter, QueueMove};
use crate::domain::JobState;
use crate::ipc::StaleQueueMoveError;
use crate::output;
use crate::queue_editor::{self, EditorMoveError};
use crate::{StokerPaths, Store, StoreError};

use super::super::{application_cli_error, print_info, print_warning, runtime, scheduler_gateway};

#[derive(Debug, Clone, Copy)]
pub(crate) struct QueueSnapshot {
    locked: bool,
}

pub(crate) fn queue_snapshot(paths: &StokerPaths) -> anyhow::Result<QueueSnapshot> {
    let store = Store::open(&paths.database)?;
    let scheduler = scheduler_gateway(paths);
    let status = runtime()?
        .block_on(application::queue::queue_status(&store, &scheduler))
        .map_err(application_cli_error)?;
    Ok(QueueSnapshot {
        locked: status.snapshot.locked,
    })
}

pub(crate) fn lock_queue(paths: &StokerPaths) -> anyhow::Result<()> {
    set_queue_lock(paths, true)
}

pub(crate) fn unlock_queue(paths: &StokerPaths) -> anyhow::Result<()> {
    set_queue_lock(paths, false)
}

pub(crate) fn set_queue_lock(paths: &StokerPaths, lock: bool) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let scheduler = scheduler_gateway(paths);
    let result = runtime()?
        .block_on(application::queue::set_queue_locked(
            &store, &scheduler, lock,
        ))
        .map_err(application_cli_error)?;
    let before = result.before;

    let state = if lock {
        if before.snapshot.locked {
            "Queue already locked."
        } else {
            "Queue locked."
        }
    } else if before.snapshot.locked {
        "Queue unlocked."
    } else {
        "Queue already unlocked."
    };
    let state_color = if state.starts_with("Queue already") {
        Color::Yellow
    } else {
        Color::Green
    };
    if lock && before.snapshot.jobs.is_empty() {
        print_warning(format!("{state} No queued jobs to reorder."));
    } else {
        println!(
            "{}",
            output::paint(state, state_color, output::stdout_color_enabled())
        );
    }
    Ok(())
}

pub(crate) fn queue_edit(paths: &StokerPaths) -> anyhow::Result<()> {
    let snapshot = queue_snapshot(paths)?;
    if !snapshot.locked {
        anyhow::bail!("Queue is unlocked. Run 'stoker queue lock' first.");
    }

    let store = Store::open(&paths.database)?;
    if store.current_mode()? == crate::domain::flow::ExecutionMode::Scheduled {
        anyhow::bail!("queue edit is unavailable in scheduled mode");
    }
    let initial_jobs = application::jobs::query_jobs(
        &store,
        &JobFilter {
            user: None,
            state: Some(JobState::Queued),
        },
    )
    .map_err(application_cli_error)?;
    if initial_jobs.is_empty() {
        print_info("No queued jobs to reorder.");
        return Ok(());
    }

    let scheduler = scheduler_gateway(paths);
    queue_editor::run_queue_editor(
        initial_jobs,
        |id, target_order| {
            runtime()
                .and_then(|runtime| {
                    runtime
                        .block_on(application::queue::move_queued(
                            &store,
                            &scheduler,
                            QueueMove { id, target_order },
                        ))
                        .map(|status| status.snapshot.jobs)
                        .map_err(application_cli_error)
                })
                .map_err(editor_move_error)
        },
        || {
            application::jobs::query_jobs(
                &store,
                &JobFilter {
                    user: None,
                    state: Some(JobState::Queued),
                },
            )
            .map_err(application_cli_error)
        },
    )
}

pub(crate) fn editor_move_error(error: anyhow::Error) -> EditorMoveError {
    if is_stale_move_error(&error) {
        EditorMoveError::Stale
    } else {
        EditorMoveError::Callback(error)
    }
}

pub(crate) fn is_stale_move_error(error: &anyhow::Error) -> bool {
    if let Some(ApplicationError::Conflict(application::Conflict::StaleQueue)) =
        error.downcast_ref::<ApplicationError>()
    {
        return true;
    }
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
