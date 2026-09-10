use std::path::{Path, PathBuf};

use stoker::application::ports::{
    JobCanceller, JobCommitter, JobCreator, JobQueries, QueueRepository, WorkingDirectoryResolver,
};
use stoker::application::{
    self, CommitSelection, CreateJobInput, DescriptionUpdate, JobFilter, PreparedJobInput,
    QueueMove, SnapshotReason,
};
use stoker::{JobState, StokerPaths, Store};

struct ExistingDirectory;

impl WorkingDirectoryResolver for ExistingDirectory {
    fn resolve_working_directory(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(path.to_path_buf())
    }
}

fn paths(root: &Path) -> StokerPaths {
    StokerPaths {
        root: root.to_path_buf(),
        database: root.join("stoker.db"),
        runs: root.join("runs"),
        lock: root.join("stoker.lock"),
        endpoint: root.join("stoker.sock"),
    }
}

fn create(store: &Store, root: &Path, name: &str, user: &str) -> stoker::Job {
    application::jobs::create_job(
        store,
        &ExistingDirectory,
        CreateJobInput {
            user: user.to_owned(),
            name: name.to_owned(),
            description: None,
            cwd: root.to_path_buf(),
            command_line: format!("echo {name}"),
        },
    )
    .unwrap()
}

#[test]
fn store_adapter_exercises_every_job_and_queue_capability() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let first = create(&store, directory.path(), "first", "alice");
    let second = create(&store, directory.path(), "second", "alice");
    let third = create(&store, directory.path(), "third", "bob");

    let updated = application::jobs::update_description(
        &store,
        DescriptionUpdate {
            id: first.id,
            description: Some("reviewed".to_owned()),
            expected_revision: 0,
        },
    )
    .unwrap();
    assert_eq!(updated.description.as_deref(), Some("reviewed"));

    let committed =
        JobCommitter::commit_jobs(&store, &CommitSelection::Jobs(vec![first.id, second.id]))
            .unwrap();
    assert_eq!(committed.len(), 2);
    let bob = JobCommitter::commit_jobs(&store, &CommitSelection::User("bob".to_owned())).unwrap();
    assert_eq!(
        bob,
        vec![application::jobs::job_detail(&store, third.id).unwrap()]
    );
    assert!(
        JobCommitter::commit_jobs(&store, &CommitSelection::All)
            .unwrap()
            .is_empty()
    );

    let snapshot = QueueRepository::set_queue_locked(&store, true).unwrap();
    assert!(snapshot.locked);
    let fourth = create(&store, directory.path(), "fourth", "alice");
    assert_eq!(
        JobCommitter::commit_jobs(&store, &CommitSelection::Jobs(vec![fourth.id]))
            .unwrap_err()
            .to_string(),
        "queue is locked"
    );
    let moved = QueueRepository::move_queued(
        &store,
        QueueMove {
            id: second.id,
            target_order: 1,
        },
    )
    .unwrap();
    assert_eq!(moved.jobs[0].id, second.id);
    assert!(
        QueueRepository::move_queued(
            &store,
            QueueMove {
                id: first.id,
                target_order: 99,
            },
        )
        .is_err()
    );
    QueueRepository::set_queue_locked(&store, false).unwrap();
    assert!(
        QueueRepository::move_queued(
            &store,
            QueueMove {
                id: first.id,
                target_order: 1,
            },
        )
        .is_err()
    );
    let cancelled = JobCanceller::cancel_not_started(&store, third.id).unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled);

    assert_eq!(
        application::jobs::query_jobs(
            &store,
            &JobFilter {
                user: Some("alice".to_owned()),
                state: Some(JobState::Queued),
            },
        )
        .unwrap()
        .len(),
        2
    );
    assert!(JobQueries::get_job(&store, uuid::Uuid::nil()).is_err());
    assert!(JobCommitter::commit_jobs(&store, &CommitSelection::Jobs(vec![first.id])).is_err());
    assert!(
        application::jobs::update_description(
            &store,
            DescriptionUpdate {
                id: first.id,
                description: None,
                expected_revision: 0,
            },
        )
        .is_err()
    );
    assert_eq!(
        JobCreator::create_job(
            &store,
            PreparedJobInput {
                user: " ".to_owned(),
                name: "invalid".to_owned(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".to_owned()],
                command_line: "echo".to_owned(),
            }
        )
        .unwrap_err()
        .to_string(),
        "job repository returned invalid data: user must not be empty"
    );
}

#[test]
fn configuration_adapter_round_trips_settings_and_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let paths = paths(directory.path());
    paths.ensure().unwrap();
    let configured = application::configuration::set_timezone(&paths, "UTC".to_owned()).unwrap();
    assert_eq!(configured.timezone.as_deref(), Some("UTC"));
    let snapshot =
        application::configuration::create_snapshot(&paths, SnapshotReason::Manual).unwrap();
    assert!(snapshot.valid);
    assert!(snapshot.created_at.is_some());
    application::configuration::unset_timezone(&paths).unwrap();
    let restored = application::configuration::restore_snapshot(&paths, &snapshot.path).unwrap();
    assert_eq!(restored.timezone.as_deref(), Some("UTC"));

    let invalid = paths.snapshot_dir().join("invalid.json");
    std::fs::write(&invalid, "not-json").unwrap();
    let listed = application::configuration::list_snapshots(&paths).unwrap();
    assert!(
        listed
            .iter()
            .any(|entry| entry.path == invalid && !entry.valid)
    );
    assert_eq!(
        application::configuration::restore_snapshot(&paths, &invalid)
            .unwrap_err()
            .code(),
        stoker::application::ApplicationErrorCode::InvalidDependencyData
    );
    assert_eq!(
        application::configuration::restore_snapshot(
            &paths,
            &paths.snapshot_dir().join("missing.json")
        )
        .unwrap_err()
        .code(),
        stoker::application::ApplicationErrorCode::NotFound
    );
}
