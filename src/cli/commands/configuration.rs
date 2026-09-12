//! User configuration command orchestration.

use anyhow::Context;
use crossterm::style::Color;

use crate::StokerPaths;
use crate::application::{self, SnapshotReason};
use crate::config::resolve_timezone;
use crate::output;

use super::super::{
    ConfigCommand, ConfigKey, application_cli_error, print_success, restore_config, select_timezone,
};

pub(crate) fn config(
    paths: &StokerPaths,
    command: ConfigCommand,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    match command {
        ConfigCommand::Set {
            key: ConfigKey::Timezone,
            value,
        } => {
            let current =
                application::configuration::configuration(paths).map_err(application_cli_error)?;
            let value = match value {
                Some(value) => value,
                None => {
                    let Some(value) = select_timezone(current.timezone.as_deref())? else {
                        return Ok(());
                    };
                    value
                }
            };
            resolve_timezone(paths, Some(&value))?;
            application::configuration::set_timezone(paths, value.clone())
                .map_err(application_cli_error)?;
            print_success(format!("Set timezone to {value}."));
        }
        ConfigCommand::Show => {
            let current =
                application::configuration::configuration(paths).map_err(application_cli_error)?;
            println!(
                "{}",
                output::paint_bold(
                    "Stoker configuration",
                    Color::Cyan,
                    output::stdout_color_enabled(),
                )
            );
            println!("File: {}", paths.config_path().display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "timezone": current.timezone,
                }))
                .context("format Stoker configuration")?
            );
        }
        ConfigCommand::Get {
            key: ConfigKey::Timezone,
        } => match application::configuration::configuration(paths)
            .map_err(application_cli_error)?
            .timezone
        {
            Some(value) => println!("timezone: {value}"),
            None => println!("timezone: <using operating system timezone>"),
        },
        ConfigCommand::Unset {
            key: ConfigKey::Timezone,
        } => {
            application::configuration::unset_timezone(paths).map_err(application_cli_error)?;
            print_success("Unset timezone; using operating system timezone.");
        }
        ConfigCommand::Restore => restore_config(paths, cli_timezone)?,
        ConfigCommand::Snapshot => {
            let snapshot =
                application::configuration::create_snapshot(paths, SnapshotReason::Manual)
                    .map_err(application_cli_error)?;
            print_success(format!(
                "Created configuration snapshot: {}.",
                snapshot.path.display()
            ));
        }
    }
    Ok(())
}
