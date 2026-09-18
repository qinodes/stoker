//! Scheduled jobs and flow command surface.
//!
//! The original `Cli` type is kept stable for existing library consumers. The
//! binary uses this parser only when a new flow/schedule command or option is
//! present, so old integrations keep the original clap schema.

use anyhow::Result;
use clap::Parser;

use crate::{StokerPaths, Store};

#[path = "extended_args.rs"]
mod extended_args;
#[path = "extended_commands.rs"]
mod extended_commands;
#[path = "extended_flow.rs"]
mod extended_flow;
#[path = "extended_output.rs"]
mod extended_output;
#[path = "extended_parsing.rs"]
mod extended_parsing;
#[path = "extended_standalone_args.rs"]
mod extended_standalone_args;

use extended_args::ExtendedCli;

pub(crate) fn maybe_run() -> Option<Result<()>> {
    let args: Vec<String> = std::env::args().collect();
    if !is_extended(&args) {
        return None;
    }
    Some(run_parsed(ExtendedCli::try_parse_from(args)))
}

fn is_extended(args: &[String]) -> bool {
    let names = [
        "flow",
        "mode",
        "run",
        "freeze",
        "unfreeze",
        "schedule",
        "draft",
        "disable",
        "enable",
        "runs",
        "occurrences",
        "recovery",
        "request",
    ];
    let mut positionals = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let argument = &args[index];
        if argument.starts_with('-') {
            if !argument.contains('=') && index + 1 < args.len() {
                let next = &args[index + 1];
                if !next.starts_with('-') {
                    index += 1;
                }
            }
        } else {
            positionals.push(argument.as_str());
        }
        index += 1;
    }
    let command = positionals.first().copied();
    let asks_for_help = args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"));
    command.is_some_and(|command| names.contains(&command))
        || asks_for_help && matches!(command, Some("create" | "jobs"))
        || args.iter().any(|arg| {
            [
                "--once-at",
                "--daily",
                "--every",
                "--first-at",
                "--schedule-timezone",
                "--flow-id",
                "--task-id",
                "--retry",
                "--dependency",
                "--mode",
                "--run",
                "--attempt",
            ]
            .contains(&arg.as_str())
        })
}

fn run_parsed(parsed: Result<ExtendedCli, clap::Error>) -> Result<()> {
    let cli = match parsed {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(anyhow::anyhow!(error.to_string())),
    };
    let paths = StokerPaths::from_env()?;
    paths.ensure()?;
    let store = Store::open(&paths.database)?;
    extended_commands::dispatch(&paths, &store, cli.command)
}
