#[test]
fn submission_public_facade_delegates_to_the_application_creation_flow() {
    let directory = tempfile::tempdir().unwrap();
    let store = stoker::Store::open(directory.path().join("stoker.db")).unwrap();
    let created = stoker::submission::create_shell_job(
        &store,
        "alice".to_owned(),
        "facade".to_owned(),
        directory.path().to_path_buf(),
        "echo facade".to_owned(),
        Some("compatible".to_owned()),
    )
    .unwrap();
    assert_eq!(created.command, ["echo", "facade"]);
    assert_eq!(created.command_line.as_deref(), Some("echo facade"));
    assert_eq!(created.description.as_deref(), Some("compatible"));
}
