//! Mapping application outcomes to stable CLI messages and colours.

use std::fmt::Display;

use crossterm::style::Color;

use crate::application::{self, ApplicationError};
use crate::output;

pub(crate) fn application_cli_error(error: ApplicationError) -> anyhow::Error {
    match error {
        ApplicationError::InvalidInput(error) => anyhow::anyhow!("--{error}"),
        ApplicationError::InvalidCommand { message } => anyhow::anyhow!("--cmd {message}"),
        ApplicationError::Conflict(application::Conflict::QueueLocked) => {
            anyhow::anyhow!("queue is locked; run 'stoker queue unlock'")
        }
        ApplicationError::Conflict(application::Conflict::QueueUnlocked) => {
            anyhow::anyhow!("queue is unlocked; run 'stoker queue lock' first")
        }
        ApplicationError::Unavailable {
            dependency: application::Dependency::Scheduler,
            ..
        } => anyhow::anyhow!("Scheduler is not running. Run `stoker start` first."),
        other => anyhow::Error::new(other),
    }
}

pub(crate) fn print_success(message: impl Display) {
    println!(
        "{}",
        output::paint(message, Color::Green, output::stdout_color_enabled())
    );
}

pub(crate) fn print_warning(message: impl Display) {
    println!(
        "{}",
        output::paint(message, Color::Yellow, output::stdout_color_enabled())
    );
}

pub(crate) fn print_info(message: impl Display) {
    println!(
        "{}",
        output::paint(message, Color::Cyan, output::stdout_color_enabled())
    );
}
