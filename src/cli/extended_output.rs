//! Output and lookup helpers for the extended Task 003 CLI.

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::{StokerPaths, Store};

use super::extended_args::LogsArgs;

pub(super) fn resolve_definition_id(store: &Store, id: &str) -> Result<String> {
    if store.get_flow(id).is_ok() {
        return Ok(id.to_owned());
    }
    let job_id = Uuid::parse_str(id).context("definition-id must be a flow ID or job UUID")?;
    store.standalone_definition(job_id)?;
    Ok(format!("standalone/{job_id}"))
}

pub(super) fn show_definition(store: &Store, id: &str, run_id: Option<Uuid>) -> Result<()> {
    if let Some(run_id) = run_id {
        let run = store.get_flow_run(run_id)?;
        let flow_id = resolve_definition_id(store, id)?;
        if run.flow_id != flow_id {
            anyhow::bail!("run does not belong to definition {id}");
        }
        return print_run(store, run_id);
    }
    let flow_id = resolve_definition_id(store, id)?;
    print_flow(&store.get_flow(&flow_id)?);
    Ok(())
}

pub(super) fn logs(paths: &StokerPaths, store: &Store, args: LogsArgs) -> Result<()> {
    let flow_id = resolve_definition_id(store, &args.id)?;
    let flow = store.get_flow(&flow_id)?;
    let task_id = if flow_id.starts_with("standalone/") {
        "job"
    } else {
        flow.tasks
            .first()
            .map(|task| task.task_id.as_str())
            .context("flow has no tasks")?
    };
    print_task_logs(
        paths,
        store,
        &flow_id,
        task_id,
        args.run,
        args.attempt,
        args.follow,
    )
}

pub(super) fn print_task_logs(
    paths: &StokerPaths,
    store: &Store,
    flow_id: &str,
    task_id: &str,
    run_id: Uuid,
    attempt: Option<u32>,
    follow: bool,
) -> Result<()> {
    let run = store.get_flow_run(run_id)?;
    if run.flow_id != flow_id {
        anyhow::bail!("run does not belong to flow");
    }
    let task = run
        .tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .context("task does not exist in run")?;
    let attempts: Vec<u32> = match attempt {
        Some(attempt) => vec![attempt],
        None => (1..=task.attempt_count).collect(),
    };
    if attempts.is_empty() && !follow {
        println!("No attempts are available for task {task_id}.");
        return Ok(());
    }
    let mut found = false;
    for attempt in &attempts {
        let directory = paths
            .runs
            .join("flows")
            .join(run_id.to_string())
            .join(task_id)
            .join(format!("attempt-{attempt}"));
        for stream in ["stdout", "stderr"] {
            let path = directory.join(format!("{stream}.log"));
            if path.is_file() {
                found = true;
                println!("--- {} ---", path.display());
                print!("{}", String::from_utf8_lossy(&std::fs::read(&path)?));
            }
        }
    }
    if follow {
        let mut lengths = std::collections::BTreeMap::new();
        for selected in &attempts {
            for stream in ["stdout", "stderr"] {
                let path = paths
                    .runs
                    .join("flows")
                    .join(run_id.to_string())
                    .join(task_id)
                    .join(format!("attempt-{selected}"))
                    .join(format!("{stream}.log"));
                let length = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                lengths.insert(path, length);
            }
        }
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let run = store.get_flow_run(run_id)?;
            let current = run
                .tasks
                .iter()
                .find(|item| item.task_id == task_id)
                .context("task does not exist in run")?;
            let selected_attempts: Vec<u32> = match attempt {
                Some(selected) => vec![selected],
                None => (1..=current.attempt_count).collect(),
            };
            for selected in selected_attempts {
                for stream in ["stdout", "stderr"] {
                    let path = paths
                        .runs
                        .join("flows")
                        .join(run_id.to_string())
                        .join(task_id)
                        .join(format!("attempt-{selected}"))
                        .join(format!("{stream}.log"));
                    let bytes = match std::fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error.into()),
                    };
                    let offset = lengths.entry(path).or_insert(0);
                    let start = usize::try_from(*offset)
                        .unwrap_or(bytes.len())
                        .min(bytes.len());
                    if start < bytes.len() {
                        print!("{}", String::from_utf8_lossy(&bytes[start..]));
                        *offset = bytes.len() as u64;
                    }
                }
            }
            if current.state.is_terminal() {
                break;
            }
        }
    } else if !found {
        let directory = paths
            .runs
            .join("flows")
            .join(run_id.to_string())
            .join(task_id);
        println!(
            "No logs are available; expected directory {}.",
            directory.display()
        );
    }
    Ok(())
}

pub(super) fn print_flow(flow: &crate::domain::FlowDefinition) {
    println!(
        "flow_id={} name={} owner={} mode={} committed={} frozen={} enabled={} graph_revision={} draft_revision={} generation={}",
        flow.flow_id,
        flow.name,
        flow.owner,
        flow.mode,
        flow.committed,
        flow.frozen,
        flow.enabled,
        flow.graph_revision,
        flow.draft_revision,
        flow.schedule_generation
    );
    if let Some(schedule) = &flow.schedule {
        println!("schedule={schedule:?}");
    }
    for task in &flow.tasks {
        let dependencies = task
            .dependencies
            .iter()
            .map(|edge| format!("{}:{}", edge.upstream_task_id, edge.status))
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "task_id={} name={} retry={} depend_mode={} depends_on={}",
            task.task_id, task.name, task.retry, task.depend_mode, dependencies
        );
    }
}

pub(super) fn print_run(store: &Store, run_id: Uuid) -> Result<()> {
    let run = store.get_flow_run(run_id)?;
    println!(
        "run_id={} flow_id={} source={} state={:?}",
        run.run_id, run.flow_id, run.source, run.state
    );
    for task in run.tasks {
        println!(
            "task_id={} state={:?} attempts={}",
            task.task_id, task.state, task.attempt_count
        );
    }
    Ok(())
}

pub(super) fn print_runs(store: &Store, flow_id: &str) -> Result<()> {
    let flow_id = resolve_definition_id(store, flow_id)?;
    let rows = store
        .list_flow_runs(&flow_id)?
        .into_iter()
        .map(|run| {
            [
                run.run_id.to_string(),
                run.flow_id,
                format!("{:?}", run.state),
                run.source,
            ]
        })
        .collect();
    print_aligned_table(["RUN_ID", "FLOW_ID", "STATE", "SOURCE"], rows);
    Ok(())
}

pub(super) fn print_occurrences(store: &Store, flow_id: &str) -> Result<()> {
    let flow_id = resolve_definition_id(store, flow_id)?;
    let rows = store
        .list_occurrences(&flow_id)?
        .into_iter()
        .map(|occurrence| {
            [
                occurrence.occurrence_id.to_string(),
                occurrence.flow_id,
                format!("{:?}", occurrence.state),
                occurrence.due_at.to_rfc3339(),
                occurrence.reason.unwrap_or_default(),
            ]
        })
        .collect();
    print_aligned_table(
        ["OCCURRENCE_ID", "FLOW_ID", "STATE", "DUE_AT_UTC", "REASON"],
        rows,
    );
    Ok(())
}

fn print_aligned_table<const N: usize>(headers: [&str; N], rows: Vec<[String; N]>) {
    let mut widths = std::array::from_fn(|index| headers[index].chars().count());
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let header = headers.map(str::to_owned);
    let separator = widths.map(|width| "-".repeat(width));
    print_table_row(&header, &widths);
    print_table_row(&separator, &widths);
    for row in &rows {
        print_table_row(row, &widths);
    }
}

fn print_table_row<const N: usize>(cells: &[String; N], widths: &[usize; N]) {
    let mut line = String::new();
    for (index, cell) in cells.iter().enumerate() {
        line.push_str(cell);
        if index + 1 < N {
            line.push_str(&" ".repeat(widths[index] - cell.chars().count() + 2));
        }
    }
    println!("{line}");
}
