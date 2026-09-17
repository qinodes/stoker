//! Output and lookup helpers for scheduled jobs and flows.

use anyhow::{Context, Result};
use chrono_tz::Tz;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::flow::{OccurrenceState, ScheduleSpec};
use crate::{StokerPaths, Store, output};

use super::extended_args::LogsArgs;

pub(super) fn show_definition(store: &Store, id: Uuid, run_id: Option<Uuid>) -> Result<()> {
    let flow_id = format!("standalone/{id}");
    store.standalone_definition(id)?;
    if let Some(run_id) = run_id {
        let run = store.get_flow_run(run_id)?;
        if run.flow_id != flow_id {
            anyhow::bail!("run does not belong to definition {id}");
        }
        return print_run(store, run_id, None);
    }
    print_flow(&store.get_flow(&flow_id)?);
    Ok(())
}

pub(super) fn show_flow_definition(
    store: &Store,
    flow_id: &str,
    run_id: Option<Uuid>,
    task_id: Option<&str>,
) -> Result<()> {
    store.get_flow(flow_id)?;
    if let Some(run_id) = run_id {
        let run = store.get_flow_run(run_id)?;
        if run.flow_id != flow_id {
            anyhow::bail!("run does not belong to flow");
        }
        return print_run(store, run_id, task_id);
    }
    print_flow(&store.get_flow(flow_id)?);
    Ok(())
}

pub(super) fn logs(paths: &StokerPaths, store: &Store, args: LogsArgs) -> Result<()> {
    let flow_id = format!("standalone/{}", args.id);
    store.standalone_definition(args.id)?;
    print_task_logs(
        paths,
        store,
        &flow_id,
        "job",
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
    let tasks = flow
        .tasks
        .iter()
        .map(|task| {
            json!({
                "task_id": task.task_id,
                "name": task.name,
                "retry": task.retry,
                "depend_mode": task.depend_mode.to_string(),
                "depends_on": task
                    .dependencies
                    .iter()
                    .map(|edge| json!({
                        "task_id": edge.upstream_task_id,
                        "status": edge.status.to_string(),
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let schedule = flow.schedule.as_ref().map(schedule_value);
    print_json(&json!({
        "flow_id": flow.flow_id,
        "name": flow.name,
        "owner": flow.owner,
        "mode": flow.mode.to_string(),
        "committed": flow.committed,
        "frozen": flow.frozen,
        "enabled": flow.enabled,
        "graph_revision": flow.graph_revision,
        "draft_revision": flow.draft_revision,
        "generation": flow.schedule_generation,
        "schedule": schedule,
        "tasks": tasks,
    }));
}

pub(super) fn print_run(store: &Store, run_id: Uuid, task_id: Option<&str>) -> Result<()> {
    let run = store.get_flow_run(run_id)?;
    let tasks = run
        .tasks
        .iter()
        .filter(|task| task_id.is_none_or(|selected| selected == task.task_id))
        .map(|task| {
            json!({
                "task_id": task.task_id,
                "state": serde_json::to_value(task.state).expect("task state is serializable"),
                "attempts": task.attempt_count,
            })
        })
        .collect::<Vec<_>>();
    if task_id.is_some() && tasks.is_empty() {
        anyhow::bail!("task does not exist in run");
    }
    print_json(&json!({
        "run_id": run.run_id,
        "flow_id": run.flow_id,
        "source": run.source,
        "state": serde_json::to_value(run.state).expect("flow state is serializable"),
        "tasks": tasks,
    }));
    Ok(())
}

fn schedule_value(schedule: &ScheduleSpec) -> Value {
    match schedule {
        ScheduleSpec::Once { at } => json!({
            "type": "once",
            "at": at.to_rfc3339(),
        }),
        ScheduleSpec::Daily { time, timezone } => json!({
            "type": "daily",
            "time": time.format("%H:%M").to_string(),
            "timezone": timezone,
        }),
    }
}

fn print_json(value: &Value) {
    let mut rendered = String::new();
    render_json(
        value,
        0,
        None,
        output::stdout_color_enabled(),
        &mut rendered,
    );
    rendered.push('\n');
    print!("{rendered}");
}

fn render_json(
    value: &Value,
    indent: usize,
    key: Option<&str>,
    colors_enabled: bool,
    rendered: &mut String,
) {
    match value {
        Value::Object(fields) => {
            rendered.push('{');
            if !fields.is_empty() {
                rendered.push('\n');
                let last = fields.len() - 1;
                for (index, (field, field_value)) in fields.iter().enumerate() {
                    rendered.push_str(&" ".repeat(indent + 2));
                    rendered.push_str(&output::paint_bold(
                        json_string(field),
                        crossterm::style::Color::Cyan,
                        colors_enabled,
                    ));
                    rendered.push_str(": ");
                    render_json(
                        field_value,
                        indent + 2,
                        Some(field),
                        colors_enabled,
                        rendered,
                    );
                    if index != last {
                        rendered.push(',');
                    }
                    rendered.push('\n');
                }
                rendered.push_str(&" ".repeat(indent));
            }
            rendered.push('}');
        }
        Value::Array(values) => {
            rendered.push('[');
            if !values.is_empty() {
                rendered.push('\n');
                let last = values.len() - 1;
                for (index, item) in values.iter().enumerate() {
                    rendered.push_str(&" ".repeat(indent + 2));
                    render_json(item, indent + 2, key, colors_enabled, rendered);
                    if index != last {
                        rendered.push(',');
                    }
                    rendered.push('\n');
                }
                rendered.push_str(&" ".repeat(indent));
            }
            rendered.push(']');
        }
        Value::String(value) => {
            let value = json_string(value);
            if matches!(key, Some("state" | "status")) {
                rendered.push('"');
                rendered.push_str(&output::paint_state_name(
                    &value[1..value.len() - 1],
                    colors_enabled,
                ));
                rendered.push('"');
            } else {
                rendered.push_str(&value);
            }
        }
        Value::Bool(value) => rendered.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => rendered.push_str(&value.to_string()),
        Value::Null => rendered.push_str("null"),
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("JSON string serialization cannot fail")
}

pub(super) fn print_flow_list(store: &Store, owner: Option<&str>) -> Result<()> {
    let flows = store.list_flows(owner)?;
    let mut rows = Vec::with_capacity(flows.len());
    for flow in flows {
        let mut status = vec![if flow.committed { "LIVE" } else { "DRAFT" }];
        if flow.frozen {
            status.push("FROZEN");
        }
        if !flow.enabled {
            status.push("DISABLED");
        }
        let active = store
            .list_flow_runs(&flow.flow_id)?
            .into_iter()
            .find(|run| !run.state.is_terminal())
            .map_or_else(|| "-".to_owned(), |run| format!("{:?}", run.state));
        let next = if flow.committed && flow.enabled && !flow.frozen {
            store
                .list_occurrences(&flow.flow_id)?
                .into_iter()
                .filter(|occurrence| occurrence.state == OccurrenceState::Pending)
                .min_by_key(|occurrence| occurrence.due_at)
                .map_or_else(
                    || "-".to_owned(),
                    |occurrence| {
                        if let Some(ScheduleSpec::Daily { timezone, .. }) = &flow.schedule
                            && let Ok(timezone) = timezone.parse::<Tz>()
                        {
                            return occurrence.due_at.with_timezone(&timezone).to_rfc3339();
                        }
                        occurrence.due_at.to_rfc3339()
                    },
                )
        } else {
            "-".to_owned()
        };
        let schedule = match &flow.schedule {
            Some(ScheduleSpec::Once { at }) => format!("once {}", at.to_rfc3339()),
            Some(ScheduleSpec::Daily { time, timezone }) => {
                format!("daily {} {timezone}", time.format("%H:%M"))
            }
            None => "-".to_owned(),
        };
        rows.push([
            flow.flow_id,
            flow.name,
            flow.owner,
            schedule,
            status.join(","),
            active,
            next,
        ]);
    }
    let is_empty = rows.is_empty();
    print_aligned_table(
        [
            "FLOW_ID", "NAME", "USER", "SCHEDULE", "STATUS", "ACTIVE", "NEXT",
        ],
        rows,
    );
    if is_empty {
        println!("No flows found.");
    }
    Ok(())
}

pub(super) fn print_runs(store: &Store, flow_id: &str) -> Result<()> {
    store.get_flow(flow_id)?;
    let rows = store
        .list_flow_runs(flow_id)?
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
    store.get_flow(flow_id)?;
    let rows = store
        .list_occurrences(flow_id)?
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

#[cfg(test)]
mod tests {
    use super::render_json;
    use serde_json::json;

    #[test]
    fn renderer_keeps_colored_states_as_quoted_json_strings() {
        let mut rendered = String::new();
        render_json(&json!({"state": "SUCCEEDED"}), 0, None, true, &mut rendered);
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains("\"state\""));
        assert!(rendered.contains(": \"\u{1b}["));
        assert!(rendered.ends_with("\n}"));
    }
}
