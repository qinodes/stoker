mod support;

use std::path::PathBuf;

use predicates::prelude::*;
use rusqlite::Connection;
use stoker::{JobState, NewJob, Store};
use support::{TestRepo, stoker_in};

fn job(name: &str, user: &str) -> NewJob {
    NewJob {
        name: name.into(),
        user: user.into(),
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
            "command: [\"python\", \"--version\", \"&&\", \"timeout\", \"/t\", \"30\"]",
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
fn jobs_list_paused_jobs_after_queue_in_their_saved_order() {
    let repo = TestRepo::new();
    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    let first = store.create_job(job("paused-first", "alice")).unwrap();
    let second = store.create_job(job("paused-second", "alice")).unwrap();
    store.commit_job(first).unwrap();
    store.commit_job(second).unwrap();
    store.pause_queued_jobs().unwrap();
    let queued = store.create_job(job("queued", "alice")).unwrap();
    store.commit_job(queued).unwrap();

    let output = stoker_in(repo.path()).args(["jobs"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.find("queued").unwrap() < stdout.find("paused-first").unwrap());
    assert!(stdout.find("paused-first").unwrap() < stdout.find("paused-second").unwrap());
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
            "queue_order  job_id  owner  name  state  time\n",
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
fn show_displays_recorded_and_execution_directories() {
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
        .stdout(predicate::str::contains("cwd:"))
        .stdout(predicate::str::contains("execution_cwd:"))
        .stdout(predicate::str::contains("execution_cwd_status: planned"))
        .stdout(predicate::str::contains("command: [\"echo\", \"ok\"]"));
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
