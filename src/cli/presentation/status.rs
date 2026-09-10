//! Rendering for scheduler, queue, and timezone status.

use std::path::Path;

use crossterm::style::Color;

use crate::application::QueueStatus;
use crate::config::{ResolvedTimezone, TimezoneSource};
use crate::output;

use super::messages::print_warning;

pub(crate) fn print_timezone_status(
    config_path: &Path,
    config_exists: bool,
    timezone: &ResolvedTimezone,
) {
    println!(
        "Display timezone: {}",
        output::paint(&timezone.name, Color::Cyan, output::stdout_color_enabled())
    );
    if timezone.source == TimezoneSource::Config || config_exists {
        println!("Timezone config: {}", config_path.display());
    } else {
        println!("Timezone config: using operating system timezone");
    }
}

pub(crate) fn print_queue_status(locked: bool) {
    let (state, color) = if locked {
        ("locked", Color::Yellow)
    } else {
        ("unlocked", Color::Green)
    };
    println!(
        "Queue: {}",
        output::paint(state, color, output::stdout_color_enabled())
    );
    if locked {
        print_warning("Scheduler will not start another queued job while the queue is locked.");
    }
}

pub(crate) fn print_scheduler_status(status: &QueueStatus) {
    match &status.scheduler {
        Some(service) => {
            println!(
                "Scheduler: {}",
                output::paint("running", Color::Green, output::stdout_color_enabled())
            );
            println!("PID: {}", service.pid);
            println!(
                "Active job: {}",
                service
                    .active_job
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".into())
            );
            println!("Queued jobs: {}", service.queued_jobs);
            print_queue_status(service.queue_locked);
        }
        None => {
            println!(
                "Scheduler: {}",
                output::paint("stopped", Color::DarkGrey, output::stdout_color_enabled())
            );
            println!("Queued jobs: {}", status.snapshot.jobs.len());
            print_queue_status(status.snapshot.locked);
        }
    }
}
