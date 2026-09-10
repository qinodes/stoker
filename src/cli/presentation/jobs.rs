//! Stable formatting for job tables and detail output.

use crossterm::style::Color;

use crate::config::ResolvedTimezone;
use crate::domain::{Job, JobState};
use crate::output;

#[cfg(test)]
pub(crate) fn format_jobs_row(columns: [&str; 7], widths: &[usize; 7]) -> String {
    columns
        .into_iter()
        .zip(widths)
        .map(|(value, width)| format!("{value:<width$}", width = *width))
        .collect::<Vec<_>>()
        .join("  ")
}

pub(crate) fn format_jobs_row_for_terminal(
    columns: [&str; 7],
    widths: &[usize; 7],
    header: bool,
    colors_enabled: bool,
) -> String {
    columns
        .into_iter()
        .enumerate()
        .zip(widths)
        .map(|((index, value), width)| {
            let padded = format!("{value:<width$}", width = *width);
            if header {
                output::paint_bold(padded, Color::Cyan, colors_enabled)
            } else if index == 4 {
                if let Ok(state) = value.parse::<JobState>() {
                    output::paint(padded, output::state_color(state), colors_enabled)
                } else {
                    padded
                }
            } else {
                padded
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

pub(crate) fn print_job(job: &Job, timezone: &ResolvedTimezone) {
    let working_directory_status = match job.state {
        JobState::Draft | JobState::Queued => "planned",
        JobState::Starting | JobState::Running | JobState::Cancelling => "active",
        _ => "source directory retained",
    };
    let command = job
        .command_line
        .clone()
        .unwrap_or_else(|| format!("{:?}", job.command));
    println!("id: {}", job.id);
    println!("name: {}", job.name);
    println!("user: {}", job.user);
    println!("description: {}", job.description.as_deref().unwrap_or("-"));
    println!("working_directory: {}", job.cwd.display());
    println!("working_directory_status: {working_directory_status}");
    println!("command: {command}");
    println!(
        "state: {}",
        output::paint_state(job.state, output::stdout_color_enabled())
    );
    println!("display_timezone: {}", timezone.name);
    println!(
        "queue_order: {}",
        job.queue_order
            .map(|order| order.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!("created_at: {}", timezone.format(job.created_at));
    println!(
        "committed_at: {}",
        format_optional_time(job.committed_at.as_ref(), timezone)
    );
    println!(
        "started_at: {}",
        format_optional_time(job.started_at.as_ref(), timezone)
    );
    println!(
        "finished_at: {}",
        format_optional_time(job.finished_at.as_ref(), timezone)
    );
    println!("exit_code: {:?}", job.exit_code);
    println!("pid: {:?}", job.pid);
    println!("failure_detail: {:?}", job.failure_detail);
}

pub(crate) fn format_optional_time(
    value: Option<&chrono::DateTime<chrono::Utc>>,
    timezone: &ResolvedTimezone,
) -> String {
    value
        .map(|value| timezone.format(*value))
        .unwrap_or_else(|| "-".into())
}
