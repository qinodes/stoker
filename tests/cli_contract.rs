use clap::{Command, CommandFactory, Parser, error::ErrorKind};
use stoker::cli::{Cli, CliCommand};

fn declared_subcommands(command: &Command) -> Vec<String> {
    command
        .get_subcommands()
        .filter(|subcommand| subcommand.get_name() != "help")
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect()
}

fn declared_arguments(command: &Command) -> Vec<String> {
    command
        .get_arguments()
        .map(|argument| argument.get_id().to_string())
        .filter(|id| id != "help" && id != "version")
        .collect()
}

fn named_subcommand<'a>(command: &'a Command, name: &str) -> &'a Command {
    command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == name)
        .unwrap_or_else(|| panic!("missing CLI subcommand {name:?}"))
}

#[test]
fn public_cli_schema_matches_the_cli_command_contract() {
    let command = Cli::command();
    assert_eq!(command.get_name(), "stoker");
    assert_eq!(
        declared_arguments(&command),
        ["timezone"],
        "top-level options changed"
    );
    assert_eq!(
        declared_subcommands(&command),
        [
            "add",
            "set-description",
            "show",
            "jobs",
            "config",
            "policy",
            "db",
            "clean",
            "update",
            "uninstall",
            "start",
            "service-run",
            "status",
            "queue",
            "stop",
            "ui",
            "ui-run",
            "commit",
            "cancel",
            "logs",
        ],
        "top-level command surface changed"
    );

    for hidden in ["service-run", "ui-run"] {
        assert!(
            named_subcommand(&command, hidden).is_hide_set(),
            "internal command {hidden:?} became user-visible"
        );
    }
    for removed in ["serve", "submit"] {
        assert!(
            command
                .get_subcommands()
                .all(|subcommand| subcommand.get_name() != removed),
            "removed command {removed:?} unexpectedly returned"
        );
    }

    assert_eq!(
        declared_arguments(named_subcommand(&command, "add")),
        ["user", "name", "description", "command"]
    );
    assert_eq!(
        declared_arguments(named_subcommand(&command, "commit")),
        ["ids", "all", "user"]
    );
    assert_eq!(
        declared_arguments(named_subcommand(&command, "logs")),
        ["id", "follow"]
    );
    assert_eq!(
        declared_subcommands(named_subcommand(&command, "config")),
        ["set", "show", "get", "unset", "restore", "snapshot"]
    );
    assert_eq!(
        declared_subcommands(named_subcommand(&command, "policy")),
        ["set", "show", "get", "unset"]
    );
    assert_eq!(
        declared_subcommands(named_subcommand(&command, "db")),
        ["check", "backup", "restore"]
    );
    assert_eq!(
        declared_arguments(named_subcommand(named_subcommand(&command, "db"), "check")),
        ["integrity"]
    );
    assert_eq!(
        declared_arguments(named_subcommand(named_subcommand(&command, "db"), "backup")),
        ["destination"]
    );
    assert_eq!(
        declared_arguments(named_subcommand(
            named_subcommand(&command, "db"),
            "restore"
        )),
        ["source", "yes"]
    );
    assert!(
        Cli::try_parse_from(["stoker", "config", "set", "log-max-bytes-per-job", "2MiB"]).is_err(),
        "scheduler policies must not be exposed through config"
    );
    assert!(
        Cli::try_parse_from(["stoker", "policy", "set", "log-max-bytes-per-job", "2MiB"]).is_ok()
    );
    assert_eq!(
        declared_subcommands(named_subcommand(&command, "queue")),
        ["lock", "edit", "unlock"]
    );
    assert_eq!(
        declared_subcommands(named_subcommand(&command, "ui")),
        ["start", "status", "stop"]
    );
}

#[test]
fn global_timezone_alias_and_commit_exclusivity_remain_parseable() {
    for timezone_option in ["--timezone", "--tz"] {
        let cli = Cli::try_parse_from(["stoker", "status", timezone_option, "UTC"]).unwrap();
        assert_eq!(cli.timezone.as_deref(), Some("UTC"));
        assert!(matches!(cli.command, CliCommand::Status));
    }

    let all = Cli::try_parse_from(["stoker", "commit", "--all"]).unwrap();
    assert!(matches!(all.command, CliCommand::Commit { all: true, .. }));
    let user = Cli::try_parse_from(["stoker", "commit", "--user", "alice"]).unwrap();
    assert!(matches!(
        user.command,
        CliCommand::Commit {
            user: Some(ref value),
            ..
        } if value == "alice"
    ));

    let conflict = Cli::try_parse_from([
        "stoker",
        "commit",
        "00000000-0000-0000-0000-000000000000",
        "--all",
    ])
    .unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::ArgumentConflict);

    let missing_selector = Cli::try_parse_from(["stoker", "commit"]).unwrap_err();
    assert_eq!(missing_selector.kind(), ErrorKind::MissingRequiredArgument);
}
