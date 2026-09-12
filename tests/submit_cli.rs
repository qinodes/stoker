mod support;

use std::path::PathBuf;

use predicates::prelude::*;
use rusqlite::Connection;
use stoker::{JobState, NewJob, Store};
use support::{TestRepo, stoker_in, stoker_with_home};

fn job(name: &str, user: &str) -> NewJob {
    NewJob {
        name: name.into(),
        user: user.into(),
        description: None,
        cwd: PathBuf::from("."),
        command: vec!["echo".into(), name.into()],
    }
}

fn finish_job(store: &Store, name: &str, user: &str, exit_code: Option<i32>) -> uuid::Uuid {
    let id = store.create_job(job(name, user)).unwrap();
    store.commit_job(id).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(id, 1).unwrap();
    store.finish(id, exit_code, None).unwrap();
    id
}

#[test]
fn add_records_absolute_cwd_as_draft() {
    let repo = TestRepo::new();
    let output = stoker_in(&repo.join("experiments/llama"))
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "lr",
            "--cmd",
            "python train.py --lr 0.0001",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Working directory:"));
    assert!(stdout.contains("Next: stoker show "));
    assert!(stdout.contains("stoker commit "));

    let home = repo.join("experiments").join(".llama-stoker-home");
    let db = Connection::open(home.join("stoker.db")).unwrap();
    let (state, cwd, command, command_line): (String, String, String, String) = db
        .query_row(
            "SELECT state, cwd, command, command_line FROM jobs LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, "DRAFT");
    assert!(std::path::Path::new(&cwd).is_absolute());
    assert!(cwd.ends_with("experiments/llama"));
    assert_eq!(command, r#"["python","train.py","--lr","0.0001"]"#);
    assert_eq!(command_line, "python train.py --lr 0.0001");
}

#[test]
fn add_and_set_description_persist_optional_text() {
    let repo = TestRepo::new();
    let output = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "described",
            "--description",
            "Initial purpose",
            "--cmd",
            "echo ok",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();

    stoker_in(repo.path())
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("description: Initial purpose"))
        .stdout(predicate::str::contains("Some(\"Initial purpose\")").not());
    stoker_in(repo.path())
        .args(["set-description", &id, "Updated purpose"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated description for job"));
    stoker_in(repo.path())
        .args(["set-description", &id, "--clear"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleared description for job"));
    stoker_in(repo.path())
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("description: -"));

    let long_description = "d".repeat(stoker::MAX_JOB_DESCRIPTION_LENGTH + 1);
    stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "too-long-description",
            "--description",
            &long_description,
            "--cmd",
            "echo ok",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--description must be 200 characters or fewer",
        ));
}

#[test]
fn binary_reports_initialization_errors_through_main_exit_path() {
    let directory = tempfile::tempdir().unwrap();
    let unusable_home = directory.path().join("home-file");
    std::fs::write(&unusable_home, "not a directory").unwrap();

    stoker_with_home(&unusable_home)
        .args(["status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn add_rejects_unquoted_multiple_command_arguments() {
    let repo = TestRepo::new();
    repo.write("tracked.txt", "changed\n");
    stoker_in(repo.path())
        .args([
            "add", "--user", "alice", "--name", "lr", "--cmd", "echo", "ok",
        ])
        .assert()
        .failure();
}

#[test]
fn add_accepts_a_non_git_directory_and_shell_command() {
    let repo = TestRepo::new();
    repo.write("tracked.txt", "changed\n");
    stoker_in(repo.path())
        .args(["add", "--user", "alice", "--name", "lr", "--cmd", "echo ok"])
        .assert()
        .success();
}

#[test]
fn add_parses_shell_command_for_show_but_preserves_raw_command() {
    let repo = TestRepo::new();
    let output = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "shell",
            "--cmd",
            "python --version && timeout /t 30",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();

    stoker_in(repo.path())
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "command: python --version && timeout /t 30",
        ));

    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let command_line: String = Connection::open(home.join("stoker.db"))
        .unwrap()
        .query_row(
            "SELECT command_line FROM jobs WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(command_line, "python --version && timeout /t 30");
}

#[test]
fn timezone_config_and_cli_override_are_reflected_in_display_output() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["config", "set", "timezone", "Asia/Tokyo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Set timezone to Asia/Tokyo."));

    let output = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "timezone-job",
            "--cmd",
            "echo ok",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();

    stoker_in(repo.path())
        .args(["show", &id, "--tz", "Asia/Taipei"])
        .assert()
        .success()
        .stdout(predicate::str::contains("display_timezone: Asia/Taipei"))
        .stdout(predicate::str::contains("created_at:").and(predicate::str::contains("+08:00")));

    stoker_in(repo.path())
        .args(["status", "--tz", "not/a-zone"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Display timezone: Asia/Tokyo"))
        .stdout(
            predicate::str::contains("Timezone config: ")
                .and(predicate::str::contains("config.json")),
        );
}

#[test]
fn config_commands_show_get_unset_and_create_manual_snapshot() {
    let repo = TestRepo::new();

    stoker_in(repo.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stoker configuration"))
        .stdout(predicate::str::contains("File:"))
        .stdout(predicate::str::contains("timezone"));
    stoker_in(repo.path())
        .args(["config", "get", "timezone"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("timezone: "));

    stoker_in(repo.path())
        .args(["config", "set", "timezone", "UTC"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["config", "get", "timezone"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timezone: UTC"));
    stoker_in(repo.path())
        .args(["config", "unset", "timezone"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unset timezone"));
    stoker_in(repo.path())
        .args(["config", "get", "timezone"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "timezone: <using operating system timezone>",
        ));
    stoker_in(repo.path())
        .args(["config", "snapshot"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created configuration snapshot:"));
}

#[test]
fn log_capacity_policy_requires_queue_lock_and_supports_defaults() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["policy", "set", "log-max-bytes-per-job", "2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("queue is unlocked"));

    stoker_in(repo.path())
        .args(["queue", "lock"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "set", "log-max-bytes-per-job", "2"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "set", "log-max-bytes-per-job", "2MiB"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("without a unit suffix"));
    stoker_in(repo.path())
        .args(["policy", "get", "log-max-bytes-per-job"])
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"(?m)^log-max-bytes-per-job: 2\r?$").unwrap());
    stoker_in(repo.path())
        .args(["policy", "unset", "log-max-bytes-per-job"])
        .assert()
        .success()
        .stdout(predicate::str::contains("built-in default"));
}

#[test]
fn runtime_policy_and_database_operations_are_available() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["queue", "lock"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "set", "termination-grace-ms", "2000"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "get", "termination-grace-ms"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2000"));
    stoker_in(repo.path())
        .args(["policy", "set", "max-runtime-ms", "1000"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "get", "max-runtime-ms"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1000"));
    stoker_in(repo.path())
        .args(["policy", "unset", "max-runtime-ms"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["policy", "get", "max-runtime-ms"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<disabled>"));
    stoker_in(repo.path())
        .args(["policy", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stoker scheduler policy"))
        .stdout(predicate::str::contains("Database:"))
        .stdout(predicate::str::contains("\"log\""))
        .stdout(predicate::str::contains("\"runtime\""));
    stoker_in(repo.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stoker configuration"))
        .stdout(predicate::str::contains("\"timezone\""))
        .stdout(predicate::str::contains("\"log\"").not())
        .stdout(predicate::str::contains("\"runtime\"").not());
    stoker_in(repo.path())
        .args(["config", "get", "max-runtime-ms"])
        .assert()
        .failure();
    stoker_in(repo.path())
        .args(["db", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("quick_check"));
    stoker_in(repo.path())
        .args(["db", "check", "--integrity"])
        .assert()
        .success()
        .stdout(predicate::str::contains("integrity_check"));

    let default_backup = stoker_in(repo.path())
        .args(["db", "backup"])
        .output()
        .unwrap();
    assert!(default_backup.status.success());
    let default_backup = String::from_utf8_lossy(&default_backup.stdout);
    let default_backup = default_backup
        .lines()
        .find_map(|line| line.strip_prefix("Created SQLite backup: "))
        .expect("default backup path is printed")
        .trim_end_matches('.')
        .to_owned();
    let default_backup_path = PathBuf::from(&default_backup);
    assert!(default_backup_path.is_file());
    assert!(
        default_backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("stoker-")
    );
    assert_eq!(
        default_backup_path.parent().unwrap().file_name().unwrap(),
        "backups"
    );

    let explicit_backup = repo.join("explicit-backup.sqlite");
    let explicit_backup_arg = explicit_backup.to_string_lossy().into_owned();
    stoker_in(repo.path())
        .args(["db", "backup", &explicit_backup_arg])
        .assert()
        .success()
        .stdout(predicate::str::contains("explicit-backup.sqlite"));
    assert!(explicit_backup.is_file());

    let marker = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "restore-marker",
            "--cmd",
            "echo marker",
        ])
        .output()
        .unwrap();
    assert!(marker.status.success());
    let marker_id = String::from_utf8_lossy(&marker.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();
    stoker_in(repo.path())
        .args(["db", "restore", &explicit_backup_arg])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires --yes"));
    stoker_in(repo.path())
        .args(["db", "restore", &explicit_backup_arg, "--yes"])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["show", &marker_id])
        .assert()
        .failure();

    stoker_in(repo.path())
        .args(["config", "show", "--db-check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn add_rejects_empty_user_and_name() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["add", "--user", "", "--name", "job", "--cmd", "echo ok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--user must not be empty"));
    stoker_in(repo.path())
        .args(["add", "--user", "alice", "--name", "", "--cmd", "echo ok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--name must not be empty"));
}

#[test]
fn add_rejects_job_names_and_users_longer_than_the_supported_limits() {
    let repo = TestRepo::new();
    let long_name = "x".repeat(stoker::domain::MAX_JOB_NAME_LENGTH + 1);
    stoker_in(repo.path())
        .args([
            "add", "--user", "alice", "--name", &long_name, "--cmd", "echo ok",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--name must be 128 characters or fewer",
        ));

    let long_user = "u".repeat(stoker::MAX_JOB_USER_LENGTH + 1);
    stoker_in(repo.path())
        .args([
            "add", "--user", &long_user, "--name", "job", "--cmd", "echo ok",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--user must be 50 characters or fewer",
        ));
}

#[test]
fn logs_explain_when_queued_or_finished_jobs_have_no_run_directory() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let finished = finish_job(&store, "finished", "alice", Some(0));
    let queued = store.create_job(job("queued", "alice")).unwrap();
    store.commit_job(queued).unwrap();

    stoker_in(repo.path())
        .args(["logs", &queued.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is QUEUED"));
    stoker_in(repo.path())
        .args(["logs", &finished.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No logs are available"));
}

#[test]
fn add_requires_user() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["add", "--name", "lr", "--cmd", "echo ok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--user"));
}

#[test]
fn submit_command_is_no_longer_available() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args([
            "submit", "--user", "alice", "--name", "job", "--cmd", "echo ok",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'submit'"));
}

#[test]
fn jobs_user_filter_excludes_other_owners() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "alice-job",
            "--cmd",
            "echo alice",
        ])
        .assert()
        .success();
    stoker_in(repo.path())
        .args([
            "add", "--user", "bob", "--name", "bob-job", "--cmd", "echo bob",
        ])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["jobs", "--user", "alice"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alice-job"))
        .stdout(predicate::str::contains("bob-job").not());
}

#[test]
fn jobs_combines_user_and_state_filters() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    finish_job(&store, "alice-failed", "alice", Some(1));
    finish_job(&store, "alice-succeeded", "alice", Some(0));
    finish_job(&store, "bob-failed", "bob", Some(1));

    stoker_in(repo.path())
        .args(["jobs", "--user", "alice", "--state", "failed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alice-failed"))
        .stdout(predicate::str::contains("alice-succeeded").not())
        .stdout(predicate::str::contains("bob-failed").not());
}

#[test]
fn jobs_lists_queued_by_queue_order_and_other_jobs_newest_first() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let queued_first = store.create_job(job("queued-first", "alice")).unwrap();
    let queued_second = store.create_job(job("queued-second", "alice")).unwrap();
    store.commit_job(queued_first).unwrap();
    store.commit_job(queued_second).unwrap();
    store.create_job(job("draft-old", "alice")).unwrap();
    store.create_job(job("draft-new", "alice")).unwrap();

    let output = stoker_in(repo.path()).args(["jobs"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.find("queued-first").unwrap() < stdout.find("queued-second").unwrap());
    assert!(stdout.find("draft-new").unwrap() < stdout.find("draft-old").unwrap());
}

#[test]
fn jobs_lists_running_before_queued_jobs() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let running = store.create_job(job("running", "alice")).unwrap();
    store.commit_job(running).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(running, 1).unwrap();
    let queued = store.create_job(job("queued", "alice")).unwrap();
    store.commit_job(queued).unwrap();

    let output = stoker_in(repo.path()).args(["jobs"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.find("running").unwrap() < stdout.find("queued").unwrap());
}

#[test]
fn clean_removes_all_terminal_jobs_and_their_logs() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let succeeded = finish_job(&store, "succeeded", "alice", Some(0));
    let failed = finish_job(&store, "failed", "alice", Some(1));
    let cancelled = store.create_job(job("cancelled", "alice")).unwrap();
    store.cancel_not_started(cancelled).unwrap();
    let lost = store.create_job(job("lost", "alice")).unwrap();
    store.commit_job(lost).unwrap();
    store.claim_next().unwrap().unwrap();
    store.mark_runtime_jobs_lost().unwrap();
    let draft = store.create_job(job("draft", "alice")).unwrap();
    let queued = store.create_job(job("queued", "alice")).unwrap();
    store.commit_job(queued).unwrap();

    for id in [succeeded, failed, cancelled, lost, draft, queued] {
        std::fs::create_dir_all(home.join("runs").join(id.to_string())).unwrap();
    }

    stoker_in(repo.path())
        .args(["clean"])
        .assert()
        .success()
        .stdout(predicate::str::contains("4"));

    assert!(
        store
            .list_jobs(None)
            .unwrap()
            .iter()
            .all(|job| { matches!(job.state, JobState::Draft | JobState::Queued) })
    );
    for id in [succeeded, failed, cancelled, lost] {
        assert!(!home.join("runs").join(id.to_string()).exists());
    }
    for id in [draft, queued] {
        assert!(home.join("runs").join(id.to_string()).exists());
    }
}

#[test]
fn jobs_empty_still_prints_header() {
    let repo = TestRepo::new();

    stoker_in(repo.path())
        .args(["jobs"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "queue_order  job_id  owner  name  state  created_at  committed_at\n",
        ));
}

#[test]
fn jobs_prints_header_before_rows() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "listed-job",
            "--cmd",
            "echo ok",
        ])
        .assert()
        .success();

    stoker_in(repo.path())
        .args(["jobs"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("queue_order  job_id"))
        .stdout(predicate::str::contains("\n-"));
}

#[test]
fn jobs_state_filter_shows_queue_order() {
    let repo = TestRepo::new();
    for (name, command) in [("queued-job", "queued"), ("draft-job", "draft")] {
        stoker_in(repo.path())
            .args([
                "add",
                "--user",
                "alice",
                "--name",
                name,
                "--cmd",
                &format!("echo {command}"),
            ])
            .assert()
            .success();
    }
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let queued = store
        .list_jobs(None)
        .unwrap()
        .into_iter()
        .find(|job| job.name == "queued-job")
        .unwrap();
    store.commit_job(queued.id).unwrap();

    stoker_in(repo.path())
        .args(["jobs", "--state", "draft"])
        .assert()
        .success()
        .stdout(predicate::str::contains("draft-job"))
        .stdout(predicate::str::contains("queued-job").not());
    stoker_in(repo.path())
        .args(["jobs", "--state", "queued"])
        .assert()
        .success()
        .stdout(predicate::str::contains("queued-job"))
        .stdout(predicate::str::contains("draft-job").not())
        .stdout(predicate::str::contains(format!(
            "\n1            {}",
            queued.id
        )));
}

#[test]
fn show_displays_the_working_directory_and_command() {
    let repo = TestRepo::new();
    stoker_in(&repo.join("experiments/llama"))
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "details-job",
            "--cmd",
            "echo ok",
        ])
        .assert()
        .success();
    let home = repo.join("experiments").join(".llama-stoker-home");
    let job = Store::open(home.join("stoker.db"))
        .unwrap()
        .list_jobs(None)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    stoker_in(&repo.join("experiments/llama"))
        .args(["show", &job.id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("working_directory:"))
        .stdout(predicate::str::contains(
            "working_directory_status: planned",
        ))
        .stdout(predicate::str::contains("command: echo ok"))
        .stdout(predicate::str::contains("execution_cwd:").not());
}

#[test]
fn logs_for_a_draft_job_explain_how_to_make_logs_available() {
    let repo = TestRepo::new();
    let output = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "waiting-logs",
            "--cmd",
            "echo ok",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();

    stoker_in(repo.path())
        .args(["logs", &id])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "Job {id} is still DRAFT; run `stoker commit {id}` before viewing its logs."
        )));
}

#[test]
fn jobs_alias_lists_submitted_job_ids() {
    let repo = TestRepo::new();
    let output = stoker_in(repo.path())
        .args([
            "add",
            "--user",
            "alice",
            "--name",
            "listed-job",
            "--cmd",
            "echo ok",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let job_id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find(|value| uuid::Uuid::parse_str(value).is_ok())
        .expect("add output contains a job ID")
        .to_owned();

    stoker_in(repo.path())
        .args(["jobs"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&job_id))
        .stdout(predicate::str::contains("listed-job"));
}

#[test]
fn version_flag_reports_package_version() {
    stoker_in(std::path::Path::new("."))
        .args(["--version"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));

    stoker_in(std::path::Path::new("."))
        .args(["-V"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
