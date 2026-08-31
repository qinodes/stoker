mod support;

use predicates::prelude::*;
use rusqlite::Connection;
use support::{TestRepo, stoker_in};

#[test]
fn submit_records_head_and_relative_cwd_as_draft() {
    let repo = TestRepo::new();
    let output = stoker_in(&repo.join("experiments/llama"))
        .args([
            "submit", "--user", "alice", "--name", "lr", "--", "echo", "ok",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let home = repo.path().parent().unwrap().join(format!(
        ".{}-stoker-home",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    let db = Connection::open(home.join("stoker.db")).unwrap();
    let (state, repository, cwd, commit): (String, String, String, String) = db
        .query_row(
            "SELECT state, repository, cwd, git_commit FROM jobs LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, "DRAFT");
    assert_eq!(
        repository,
        repo.path().canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(cwd, "experiments/llama");
    assert!(!commit.is_empty());
}

#[test]
fn submit_rejects_dirty_repository() {
    let repo = TestRepo::new();
    repo.write("tracked.txt", "changed\n");
    stoker_in(repo.path())
        .args([
            "submit", "--user", "alice", "--name", "lr", "--", "echo", "ok",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
}

#[test]
fn submit_requires_user() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args(["submit", "--name", "lr", "--", "echo", "ok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--user"));
}

#[test]
fn ps_user_filter_excludes_other_owners() {
    let repo = TestRepo::new();
    stoker_in(repo.path())
        .args([
            "submit",
            "--user",
            "alice",
            "--name",
            "alice-job",
            "--",
            "echo",
            "alice",
        ])
        .assert()
        .success();
    stoker_in(repo.path())
        .args([
            "submit", "--user", "bob", "--name", "bob-job", "--", "echo", "bob",
        ])
        .assert()
        .success();
    stoker_in(repo.path())
        .args(["ps", "--user", "alice"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alice-job"))
        .stdout(predicate::str::contains("bob-job").not());
}
