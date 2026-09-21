use std::fs;

use assert_cmd::Command;
use chrono::{Duration, NaiveTime, Utc};
use predicates::prelude::*;

use stoker::domain::flow::{
    Dependency, DependencyMode, DependencyStatus, ExecutionMode, SchedulePeriod,
    SchedulePeriodUnit, ScheduleSpec,
};
use stoker::flow_source::{
    FlowSourceCwd, FlowSourceSchedule, FlowSourceTask, canonical_hash, format_document,
    parse_document, resolve_document, resolve_workspace_document,
};
use stoker::store::{FlowSourceMode, FlowTaskInput};
use stoker::{NewJob, Store};

fn cli(home: &std::path::Path, cwd: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("stoker").unwrap();
    command
        .env("STOKER_HOME", home)
        .env("NO_COLOR", "1")
        .current_dir(cwd);
    command
}

fn scheduled_store(root: &std::path::Path) -> Store {
    let store = Store::open(root.join("home").join("stoker.db")).unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
    store
}

fn create_flow(store: &Store, cwd: &std::path::Path, id: &str, schedule: ScheduleSpec) {
    store
        .create_flow(id.into(), format!("name-{id}"), "alice".into(), schedule)
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: id.into(),
            task_id: "root".into(),
            name: "root".into(),
            cwd: cwd.to_string_lossy().into_owned(),
            command: "echo root".into(),
            retry: 1,
            dependencies: vec![],
            depend_mode: DependencyMode::All,
        })
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: id.into(),
            task_id: "cleanup".into(),
            name: "cleanup".into(),
            cwd: cwd.to_string_lossy().into_owned(),
            command: "echo cleanup".into(),
            retry: 2,
            dependencies: vec![Dependency {
                upstream_task_id: "root".into(),
                status: DependencyStatus::Failed,
            }],
            depend_mode: DependencyMode::Any,
        })
        .unwrap();
    store.commit_flow(id).unwrap();
}

#[test]
fn store_export_is_consistent_complete_and_excludes_drafts_and_hidden_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "once",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    create_flow(
        &store,
        directory.path(),
        "daily",
        ScheduleSpec::Daily {
            time: NaiveTime::from_hms_opt(23, 30, 0).unwrap(),
            timezone: "Asia/Tokyo".into(),
        },
    );
    create_flow(
        &store,
        directory.path(),
        "periodic",
        ScheduleSpec::Periodic {
            every: SchedulePeriod {
                value: 15,
                unit: SchedulePeriodUnit::Minutes,
            },
            first_at: Some(Utc::now() + Duration::hours(3)),
        },
    );
    store
        .create_flow(
            "draft".into(),
            "draft".into(),
            "alice".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(4),
            },
        )
        .unwrap();
    let standalone = store
        .create_job(NewJob {
            name: "hidden".into(),
            user: "alice".into(),
            description: None,
            cwd: directory.path().into(),
            command: vec!["echo".into(), "hidden".into()],
        })
        .unwrap();
    store
        .configure_standalone(
            standalone,
            ExecutionMode::Scheduled,
            Some(ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(5),
            }),
            0,
        )
        .unwrap();
    store.commit_job(standalone).unwrap();

    store.freeze_flow("once").unwrap();
    store
        .set_flow_task_draft(
            "once",
            "root",
            Some("echo draft-only"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let document = store.export_flow_source().unwrap();
    assert_eq!(document.base.revision, 3);
    assert_eq!(document.base.hash, canonical_hash(&document).unwrap());
    assert_eq!(
        document
            .flows
            .iter()
            .map(|flow| flow.id.as_str())
            .collect::<Vec<_>>(),
        ["once", "daily", "periodic"]
    );
    assert!(!document.flows.iter().any(|flow| flow.id == "draft"));
    assert!(
        !document
            .flows
            .iter()
            .any(|flow| flow.id.starts_with("standalone/"))
    );
    assert_eq!(document.flows[0].tasks[0].command, "echo root");
    assert_eq!(document.flows[0].tasks[1].retry, 2);
    assert_eq!(document.flows[0].tasks[1].depend_mode, DependencyMode::Any);
    assert_eq!(
        document.flows[0].tasks[1].depends_on[0].status,
        DependencyStatus::Failed
    );
    assert!(matches!(
        document.flows[0].schedule,
        FlowSourceSchedule::Once { .. }
    ));
    assert!(matches!(
        document.flows[1].schedule,
        FlowSourceSchedule::Daily { .. }
    ));
    assert!(matches!(
        document.flows[2].schedule,
        FlowSourceSchedule::Periodic { .. }
    ));
    assert!(matches!(
        document.flows[0].tasks[0].cwd,
        Some(FlowSourceCwd::Path(_))
    ));
}

#[test]
fn workspace_document_resolution_uses_the_server_selected_workspace_root() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "browser-content",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let mut document = store.export_flow_source().unwrap();
    document.flows[0].tasks[0].cwd = Some(FlowSourceCwd::Path(".".into()));

    let resolved = resolve_workspace_document(&document, directory.path()).unwrap();
    assert_eq!(
        std::path::Path::new(&resolved[0].tasks[0].cwd)
            .canonicalize()
            .unwrap(),
        directory.path().canonicalize().unwrap()
    );
}

#[test]
fn cli_export_writes_reusable_json_preserves_other_files_and_reuses_same_target() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "exported",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let output = directory.path().join("exports");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep.txt"), b"keep").unwrap();

    cli(&home, directory.path())
        .args(["flow", "export", "--dir"])
        .arg(&output)
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported 1 flow(s) at revision 1"));
    cli(&home, directory.path())
        .args(["flow", "export", "--dir"])
        .arg(&output)
        .assert()
        .success();

    assert_eq!(fs::read(output.join("keep.txt")).unwrap(), b"keep");
    let json_files = fs::read_dir(&output)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    assert_eq!(json_files.len(), 1);
    let bytes = fs::read(&json_files[0]).unwrap();
    let parsed = parse_document(&bytes).unwrap();
    assert_eq!(parsed.flows[0].id, "exported");
    let resolved = resolve_document(&parsed, &json_files[0]).unwrap();
    assert_eq!(resolved[0].tasks[0].command, "echo root");
    assert_eq!(
        std::path::Path::new(&resolved[0].tasks[0].cwd)
            .canonicalize()
            .unwrap(),
        directory.path().canonicalize().unwrap()
    );
    assert!(bytes.ends_with(b"\n"));
    assert!(
        fs::read_dir(&output)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
    );
}

#[test]
fn source_mode_requires_safe_workspace_and_gates_definition_writes() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "managed",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    assert_eq!(store.flow_source_state().unwrap().revision, 1);

    let unlocked = store
        .set_flow_source_mode(FlowSourceMode::Sync)
        .unwrap_err();
    assert!(unlocked.to_string().contains("queue is unlocked"));
    store.lock_queue().unwrap();
    let state = store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    assert_eq!(state.mode, FlowSourceMode::Sync);
    assert!(store.queue_locked().unwrap());
    let error = store.disable_flow("managed", false).unwrap_err();
    assert!(error.to_string().contains("managed by sync source mode"));
    assert!(store.list_flows(None).is_ok());

    let same = store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    assert_eq!(same.revision, state.revision);
    store.set_flow_source_mode(FlowSourceMode::Manual).unwrap();
    store.unlock_queue().unwrap();
    store.disable_flow("managed", false).unwrap();
    assert_eq!(store.flow_source_state().unwrap().revision, 2);
}

#[test]
fn source_mode_rejects_uncommitted_and_frozen_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    store
        .create_flow(
            "draft".into(),
            "draft".into(),
            "alice".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(2),
            },
        )
        .unwrap();
    store.lock_queue().unwrap();
    let error = store
        .set_flow_source_mode(FlowSourceMode::Sync)
        .unwrap_err();
    assert!(error.to_string().contains("1 uncommitted"));
}

#[test]
fn cli_source_mode_keeps_queue_locked_and_reports_state() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    store.lock_queue().unwrap();
    cli(&home, directory.path())
        .args(["flow", "source-mode", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Flow source mode is sync"))
        .stdout(predicate::str::contains("Queue remains locked"));
    assert!(store.queue_locked().unwrap());
}

#[test]
fn cli_snapshot_works_in_manual_and_sync_modes_without_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "snapshotted",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    cli(&home, directory.path())
        .args(["flow", "snapshot"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Snapshotted 1 flow(s)"));
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    cli(&home, directory.path())
        .args(["flow", "snapshot"])
        .assert()
        .success();
    assert_eq!(
        fs::read_dir(home.join("flows/snapshots")).unwrap().count(),
        1
    );
}

#[test]
fn sync_dry_run_update_noop_stale_and_remove_are_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "managed",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let base = store.export_flow_source().unwrap();
    let mut changed = base.clone();
    changed.flows[0].tasks[0].command = "echo changed".into();
    let definition_path = directory.path().join("flows.json");
    let resolved = resolve_document(&changed, &definition_path).unwrap();
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();

    let dry = store
        .sync_flow_source(directory.path(), &changed, &resolved, true)
        .unwrap();
    assert!(dry.changed);
    assert_eq!(dry.diff.updated, 1);
    assert_eq!(store.flow_source_state().unwrap().revision, 1);
    assert!(!directory.path().join("flows/snapshots").exists());

    let applied = store
        .sync_flow_source(directory.path(), &changed, &resolved, false)
        .unwrap();
    assert_eq!(applied.revision, 2);
    assert_eq!(
        store.get_flow("managed").unwrap().tasks[0].command,
        "echo changed"
    );
    assert_eq!(
        fs::read_dir(directory.path().join("flows/snapshots"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(directory.path().join("flows/sources"))
            .unwrap()
            .count(),
        1
    );

    let noop = store
        .sync_flow_source(directory.path(), &changed, &resolved, false)
        .unwrap();
    assert!(!noop.changed);
    assert_eq!(noop.revision, 2);
    assert_eq!(
        fs::read_dir(directory.path().join("flows/snapshots"))
            .unwrap()
            .count(),
        1
    );

    let mut stale = base;
    stale.flows[0].name = "stale change".into();
    let stale_resolved = resolve_document(&stale, &definition_path).unwrap();
    let error = store
        .sync_flow_source(directory.path(), &stale, &stale_resolved, false)
        .unwrap_err();
    assert!(error.to_string().contains("stale Flow definition"));

    let mut removal = store.export_flow_source().unwrap();
    removal.flows.clear();
    let removal_resolved = resolve_document(&removal, &definition_path).unwrap();
    let removed = store
        .sync_flow_source(directory.path(), &removal, &removal_resolved, false)
        .unwrap();
    assert_eq!(removed.diff.removed, 1);
    assert!(store.list_flows(None).unwrap().is_empty());
    assert!(store.queue_locked().unwrap());
}

#[test]
fn cli_sync_reports_diff_and_keeps_queue_locked() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    let mut document = store.export_flow_source().unwrap();
    document.flows.push(stoker::flow_source::FlowSourceFlow {
        id: "added".into(),
        name: "added".into(),
        owner: "alice".into(),
        enabled: true,
        schedule: FlowSourceSchedule::Once {
            at: (Utc::now() + Duration::hours(2)).to_rfc3339(),
        },
        tasks: vec![FlowSourceTask {
            id: "task".into(),
            name: "task".into(),
            cwd: None,
            command: "echo added".into(),
            retry: 0,
            depend_mode: DependencyMode::All,
            depends_on: vec![],
        }],
    });
    let path = directory.path().join("desired.json");
    fs::write(&path, format_document(&document).unwrap()).unwrap();
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&path)
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run: +1 ~0 -0 =0"));
    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Synced: +1 ~0 -0 =0"))
        .stdout(predicate::str::contains("queue unlock"));
    assert!(store.queue_locked().unwrap());
}

#[test]
fn cli_sync_rejects_terminal_controls_in_flow_display_fields() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    let mut document = store.export_flow_source().unwrap();
    document.flows.push(stoker::flow_source::FlowSourceFlow {
        id: "controlled".into(),
        name: "safe\u{1b}[2Jinjected".into(),
        owner: "alice".into(),
        enabled: true,
        schedule: FlowSourceSchedule::Once {
            at: (Utc::now() + Duration::hours(2)).to_rfc3339(),
        },
        tasks: vec![FlowSourceTask {
            id: "task".into(),
            name: "task".into(),
            cwd: None,
            command: "echo safe".into(),
            retry: 0,
            depend_mode: DependencyMode::All,
            depends_on: vec![],
        }],
    });
    let path = directory.path().join("controlled.json");
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();

    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&path)
        .arg("--dry-run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("control character"));
    assert_eq!(store.flow_source_state().unwrap().revision, 0);
}

#[test]
fn cli_flow_create_rejects_terminal_controls_before_persisting_a_draft() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    drop(store);

    for (flow_id, name, owner) in [
        ("flow\u{1b}[2J", "flow", "alice"),
        ("flow", "flow\u{1b}[2J", "alice"),
        ("flow", "flow", "alice\u{1b}[2J"),
    ] {
        cli(&home, directory.path())
            .args([
                "flow",
                "create",
                flow_id,
                "--user",
                owner,
                "--name",
                name,
                "--once-at",
                "2099-01-01T00:00:00Z",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("control character"));
    }

    assert!(
        Store::open(home.join("stoker.db"))
            .unwrap()
            .list_flows(None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cli_flow_task_add_rejects_terminal_controls_before_persisting_a_draft() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    drop(store);
    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "task-controls",
            "--user",
            "alice",
            "--name",
            "task-controls",
            "--once-at",
            "2099-01-01T00:00:00Z",
        ])
        .assert()
        .success();

    for (task_id, name) in [("task\u{1b}[2J", "task"), ("task", "task\u{1b}[2J")] {
        cli(&home, directory.path())
            .args([
                "flow",
                "task",
                "add",
                "task-controls",
                task_id,
                "--name",
                name,
                "--cmd",
                "echo safe",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("control character"));
    }

    assert!(
        Store::open(home.join("stoker.db"))
            .unwrap()
            .get_flow("task-controls")
            .unwrap()
            .tasks
            .is_empty()
    );
}

#[test]
fn cli_validation_errors_escape_terminal_controls_on_stderr() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    let mut document = store.export_flow_source().unwrap();
    let valid_flow = stoker::flow_source::FlowSourceFlow {
        id: "flow".into(),
        name: "flow".into(),
        owner: "alice".into(),
        enabled: true,
        schedule: FlowSourceSchedule::Once {
            at: (Utc::now() + Duration::hours(2)).to_rfc3339(),
        },
        tasks: vec![FlowSourceTask {
            id: "task".into(),
            name: "task".into(),
            cwd: None,
            command: "echo safe".into(),
            retry: 0,
            depend_mode: DependencyMode::All,
            depends_on: vec![],
        }],
    };
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();

    let mut reserved = valid_flow.clone();
    reserved.id = "standalone/safe\u{1b}[2J".into();
    document.flows = vec![reserved];
    let reserved_path = directory.path().join("reserved-control.json");
    fs::write(
        &reserved_path,
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();

    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&reserved_path)
        .arg("--dry-run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("\u{1b}").not());

    let mut missing_dependency = valid_flow;
    missing_dependency.tasks[0].depends_on = vec![stoker::flow_source::FlowSourceDependency {
        task_id: "missing\u{1b}[2J".into(),
        status: DependencyStatus::Succeeded,
    }];
    document.flows = vec![missing_dependency];
    let dependency_path = directory.path().join("dependency-control.json");
    fs::write(
        &dependency_path,
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();

    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&dependency_path)
        .arg("--dry-run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("\u{1b}").not());
}

#[test]
fn parser_rejects_terminal_controls_in_every_table_display_field() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    let mut document = store.export_flow_source().unwrap();
    document.flows.push(stoker::flow_source::FlowSourceFlow {
        id: "flow".into(),
        name: "flow".into(),
        owner: "alice".into(),
        enabled: true,
        schedule: FlowSourceSchedule::Once {
            at: (Utc::now() + Duration::hours(2)).to_rfc3339(),
        },
        tasks: vec![FlowSourceTask {
            id: "task".into(),
            name: "task".into(),
            cwd: None,
            command: "echo safe".into(),
            retry: 0,
            depend_mode: DependencyMode::All,
            depends_on: vec![],
        }],
    });
    let base = serde_json::to_value(document).unwrap();
    let mut cases = Vec::new();
    for (label, path) in [
        ("flow id", ["flows", "0", "id"]),
        ("flow name", ["flows", "0", "name"]),
        ("flow owner", ["flows", "0", "owner"]),
        ("task id", ["flows", "0", "tasks"]),
        ("task name", ["flows", "0", "tasks"]),
    ] {
        let mut value = base.clone();
        match label {
            "flow id" | "flow name" | "flow owner" => {
                value[path[0]][0][path[2]] = serde_json::json!("safe\u{1b}[2J");
            }
            "task id" => {
                value[path[0]][0][path[2]][0]["id"] = serde_json::json!("safe\u{1b}[2J");
            }
            "task name" => {
                value[path[0]][0][path[2]][0]["name"] = serde_json::json!("safe\u{1b}[2J");
            }
            _ => unreachable!(),
        }
        cases.push((label, value));
    }

    for (label, value) in cases {
        let error = parse_document(&serde_json::to_vec(&value).unwrap())
            .expect_err(label)
            .to_string();
        assert!(
            error.contains("control character"),
            "{label} returned unexpected error: {error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn unix_manual_task_update_preserves_backslashes_in_cwd() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "unix-cwd",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    store.freeze_flow("unix-cwd").unwrap();
    let cwd = directory.path().join(r"back\slash");
    fs::create_dir(&cwd).unwrap();

    cli(&home, directory.path())
        .args(["flow", "task", "update", "unix-cwd", "root", "--cwd"])
        .arg(&cwd)
        .assert()
        .success();

    let updated = store.get_flow("unix-cwd").unwrap();
    assert_eq!(updated.tasks[0].cwd, cwd.to_string_lossy());
}

#[test]
fn flow_list_escapes_terminal_controls_from_legacy_database_rows() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "legacy",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let connection = rusqlite::Connection::open(home.join("stoker.db")).unwrap();
    connection
        .execute(
            "UPDATE flow_definitions SET name = ?1 WHERE flow_id = 'legacy'",
            ["safe\u{1b}[2Jinjected"],
        )
        .unwrap();

    cli(&home, directory.path())
        .args(["flow", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}").not())
        .stdout(predicate::str::contains(r"safe\u{1b}[2Jinjected"));
}

#[cfg(windows)]
#[test]
fn windows_public_cli_export_sync_round_trip_is_a_noop() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let store = scheduled_store(directory.path());
    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "roundtrip",
            "--user",
            "alice",
            "--name",
            "roundtrip",
            "--once-at",
            "2099-01-01T00:00:00Z",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "task",
            "add",
            "roundtrip",
            "task",
            "--name",
            "task",
            "--cmd",
            "echo roundtrip",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["flow", "commit", "roundtrip"])
        .assert()
        .success();
    let export_directory = directory.path().join("exports");
    cli(&home, directory.path())
        .args(["flow", "export", "--dir"])
        .arg(&export_directory)
        .assert()
        .success();
    let definition = fs::read_dir(&export_directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    let before = store.get_flow("roundtrip").unwrap();

    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&definition)
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run: +0 ~0 -0 =1"));
    cli(&home, directory.path())
        .args(["flow", "sync"])
        .arg(&definition)
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes: +0 ~0 -0 =1"));

    let after = store.get_flow("roundtrip").unwrap();
    assert_eq!(after.graph_revision, before.graph_revision);
    assert_eq!(store.flow_source_state().unwrap().revision, 1);
    assert!(!home.join("flows/snapshots").exists());
    assert!(!home.join("flows/sources").exists());
}

#[test]
fn concurrent_syncs_from_one_base_allow_only_one_changed_definition() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "race",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let base = store.export_flow_source().unwrap();
    let mut first = base.clone();
    first.flows[0].name = "first".into();
    let mut second = base;
    second.flows[0].name = "second".into();
    let path = directory.path().join("race.json");
    let first_resolved = resolve_document(&first, &path).unwrap();
    let second_resolved = resolve_document(&second, &path).unwrap();
    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    let root = directory.path().to_path_buf();
    let one_store = store.clone();
    let one_root = root.clone();
    let one = std::thread::spawn(move || {
        one_store.sync_flow_source(&one_root, &first, &first_resolved, false)
    });
    let two_store = store.clone();
    let two = std::thread::spawn(move || {
        two_store.sync_flow_source(&root, &second, &second_resolved, false)
    });
    let results = [one.join().unwrap(), two.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("stale")))
            .count(),
        1
    );
}

#[test]
fn sync_gate_and_artifact_failures_leave_database_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let store = scheduled_store(directory.path());
    create_flow(
        &store,
        directory.path(),
        "rollback",
        ScheduleSpec::Once {
            at: Utc::now() + Duration::hours(2),
        },
    );
    let mut source = store.export_flow_source().unwrap();
    source.flows[0].name = "must not persist".into();
    let definition_path = directory.path().join("rollback.json");
    let resolved = resolve_document(&source, &definition_path).unwrap();
    let manual_error = store
        .sync_flow_source(directory.path(), &source, &resolved, false)
        .unwrap_err();
    assert!(manual_error.to_string().contains("source mode sync"));

    store.lock_queue().unwrap();
    store.set_flow_source_mode(FlowSourceMode::Sync).unwrap();
    let blocked_root = directory.path().join("not-a-directory");
    fs::write(&blocked_root, b"file").unwrap();
    let revision = store.flow_source_state().unwrap().revision;
    let artifact_error = store
        .sync_flow_source(&blocked_root, &source, &resolved, false)
        .unwrap_err();
    assert!(artifact_error.to_string().contains("could not access"));
    assert_eq!(store.flow_source_state().unwrap().revision, revision);
    assert_eq!(store.get_flow("rollback").unwrap().name, "name-rollback");

    store.unlock_queue().unwrap();
    let unlocked_error = store
        .sync_flow_source(directory.path(), &source, &resolved, true)
        .unwrap_err();
    assert!(unlocked_error.to_string().contains("queue is unlocked"));
}

#[test]
fn flow_help_documents_declarative_commands_and_no_color_is_plain() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    cli(&home, directory.path())
        .args(["flow", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("snapshot"))
        .stdout(predicate::str::contains("source-mode"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("\u{1b}[").not());
    cli(&home, directory.path())
        .args(["flow", "sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<DEFINITION_FILE>"))
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn cli_export_requires_scheduled_mode_and_help_documents_directory() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    cli(&home, directory.path())
        .args(["flow", "export", "--dir", "exports"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "flow export is only available in scheduled mode",
        ));
    cli(&home, directory.path())
        .args(["flow", "export", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dir <OUTPUT_DIRECTORY>"));
}
