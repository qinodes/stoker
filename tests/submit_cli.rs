mod support;

use predicates::prelude::*;
use rusqlite::Connection;
use stoker::Store;
use support::{TestRepo, stoker_in};

#[test]
fn submit_records_absolute_cwd_as_draft() {
    let repo = TestRepo::new();
    let output = stoker_in(&repo.join("experiments/llama"))
        .args([
            "submit", "--user", "alice", "--name", "lr", "--cmd", "python", "train.py", "--lr",
            "0.0001",
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
    let (state, cwd, command): (String, String, String) = db
        .query_row("SELECT state, cwd, command FROM jobs LIMIT 1", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(state, "DRAFT");
    assert!(std::path::Path::new(&cwd).is_absolute());
    assert!(cwd.ends_with("experiments/llama"));
    assert_eq!(command, r#"["python","train.py","--lr","0.0001"]"#);
}

#[test]
fn submit_accepts_a_non_git_directory() {
    let repo = TestRepo::new();
    repo.write("tracked.txt", "changed\n");
    stoker_in(repo.path())
        .args([
            "submit", "--user", "alice", "--name", "lr", "--cmd", "echo", "ok",
        ])
        .assert()
        .success();
}

#[test]
fn submit_requires_user() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["submit", "--name", "lr", "--cmd", "echo", "ok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--user"));
}

#[test]
fn jobs_user_filter_excludes_other_owners() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args([
            "submit",
            "--user",
            "alice",
            "--name",
            "alice-job",
            "--cmd",
            "echo",
            "alice",
        ])
        .assert()
        .success();
    stoker_in(repo.path())
        .args([
            "submit", "--user", "bob", "--name", "bob-job", "--cmd", "echo", "bob",
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
            "submit",
            "--user",
            "alice",
            "--name",
            "listed-job",
            "--cmd",
            "echo",
            "ok",
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
                "submit", "--user", "alice", "--name", name, "--cmd", "echo", command,
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
            "submit",
            "--user",
            "alice",
            "--name",
            "details-job",
            "--cmd",
            "echo",
            "ok",
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
            "submit",
            "--user",
            "alice",
            "--name",
            "listed-job",
            "--cmd",
            "echo",
            "ok",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let job_id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find(|value| uuid::Uuid::parse_str(value).is_ok())
        .expect("submit output contains a job ID")
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
