use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

fn assert_sources_avoid(root: &Path, forbidden: &[&str]) {
    for path in rust_sources(root) {
        let source = fs::read_to_string(&path).unwrap();
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} crosses an architecture boundary through {pattern:?}",
                path.display()
            );
        }
    }
}

fn assert_source_avoids(path: &Path, forbidden: &[&str]) {
    let source = fs::read_to_string(path).unwrap();
    for pattern in forbidden {
        assert!(
            !source.contains(pattern),
            "{} crosses an architecture boundary through {pattern:?}",
            path.display()
        );
    }
}

fn combined_sources(root: &Path) -> String {
    rust_sources(root)
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn production_source(path: &Path) -> String {
    let source = fs::read_to_string(path).unwrap();
    let test_module = source
        .match_indices("#[cfg(test)]")
        .find_map(|(index, marker)| {
            let remainder = &source[index + marker.len()..];
            remainder
                .trim_start_matches(['\r', '\n', ' ', '\t'])
                .starts_with("mod ")
                .then_some(index)
        });
    source[..test_module.unwrap_or(source.len())].to_owned()
}

#[test]
fn all_production_modules_stay_bounded_and_avoid_miscellaneous_bags() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden_names = ["common.rs", "context.rs", "helpers.rs", "utils.rs"];

    for path in rust_sources(&source_root) {
        let file_name = path.file_name().and_then(|value| value.to_str()).unwrap();
        assert!(
            !forbidden_names.contains(&file_name),
            "{} is a miscellaneous dependency bag",
            path.display()
        );
        let lines = production_source(&path).lines().count();
        assert!(
            lines <= 500,
            "{} has {lines} production lines; split it by ownership",
            path.display()
        );
    }
}

#[test]
fn public_facades_do_not_reown_framework_or_persistence_implementations() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let library = production_source(&source_root.join("lib.rs"));
    assert!(
        !library.contains("fn "),
        "lib.rs should only expose modules and re-exports"
    );

    for file in ["config.rs", "domain.rs", "ipc.rs", "store.rs", "ui.rs"] {
        let facade = production_source(&source_root.join(file));
        for forbidden in [
            "axum::",
            "rusqlite::",
            "TcpListener",
            "tokio::runtime",
            "std::process::Command",
        ] {
            assert!(
                !facade.contains(forbidden),
                "public façade {file} reowns implementation through {forbidden}"
            );
        }
    }
}

#[test]
fn domain_has_no_io_runtime_or_framework_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain");
    assert_sources_avoid(
        &root,
        &[
            "anyhow::",
            "axum::",
            "clap::",
            "rusqlite::",
            "tokio::",
            "std::env",
            "std::fs",
            "std::net",
            "std::process",
            "stdout()",
            "stderr()",
            "Runtime::",
            "from_env()",
            "ServiceClient",
            "Store::open",
            "crate::application",
            "crate::config",
            "crate::ipc",
            "crate::service",
            "crate::store",
            "crate::ui",
        ],
    );
}

#[test]
fn application_has_no_adapter_runtime_or_hidden_context_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application");
    assert_sources_avoid(
        &root,
        &[
            "anyhow::",
            "axum::",
            "clap::",
            "rusqlite::",
            "tokio::",
            "std::env",
            "std::fs",
            "std::net",
            "std::process",
            "crate::cli",
            "crate::config",
            "crate::ipc",
            "crate::service",
            "crate::store",
            "crate::ui",
            "ApplicationContext",
            "ServiceLocator",
        ],
    );
}

#[test]
fn scheduler_runtime_has_no_ipc_wire_dependencies() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "crate::ipc",
        "IpcRequest",
        "IpcResponse",
        "ServiceStatus",
        "LogStream",
    ];
    assert_source_avoids(&source_root.join("scheduler.rs"), &forbidden);
    assert_sources_avoid(&source_root.join("scheduler"), &forbidden);
}

#[test]
fn scheduler_and_service_have_explicit_stage_five_module_boundaries() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        "scheduler/control.rs",
        "scheduler/runner.rs",
        "scheduler/execution.rs",
        "scheduler/cancellation.rs",
        "scheduler/status.rs",
        "scheduler/logs.rs",
        "service/dispatch.rs",
        "service/error_mapping.rs",
        "service/log_stream.rs",
        "service/transport/mod.rs",
        "service/transport/unix.rs",
        "service/transport/windows.rs",
    ] {
        assert!(source_root.join(file).is_file(), "missing module {file}");
    }
}

#[test]
fn ipc_client_uses_typed_errors_and_does_not_own_console_output() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let client = fs::read_to_string(source_root.join("ipc/client.rs")).unwrap();
    for forbidden in [
        "stdout()",
        "stderr()",
        "print!",
        "println!",
        "eprint!",
        "eprintln!",
    ] {
        assert!(!client.contains(forbidden), "IPC client owns {forbidden}");
    }

    let adapter = production_source(&source_root.join("adapters/scheduler.rs"));
    for forbidden in [
        "message.contains",
        "message.starts_with",
        "message.ends_with",
    ] {
        assert!(
            !adapter.contains(forbidden),
            "scheduler adapter classifies errors through {forbidden}"
        );
    }
    assert!(adapter.contains("IpcErrorCode::"));
    assert!(adapter.contains("impl SchedulerStatusGateway for ServiceClient"));
    assert!(adapter.contains("impl SchedulerLogGateway for ServiceClient"));
}

#[test]
fn config_paths_do_not_own_workspace_or_persistence_side_effects() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config");
    assert_source_avoids(
        &source_root.join("paths.rs"),
        &[
            "std::fs",
            "OpenOptions",
            "create_dir_all",
            "remove_file",
            "serde_json",
            "rusqlite",
        ],
    );
}

#[test]
fn store_runtime_operations_do_not_own_schema_migration() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store");
    let forbidden = [
        "PRAGMA user_version",
        "ALTER TABLE",
        "CREATE TABLE",
        "migrations::",
    ];
    for file in [
        "description.rs",
        "jobs.rs",
        "mapping.rs",
        "queue.rs",
        "transition.rs",
    ] {
        assert_source_avoids(&source_root.join(file), &forbidden);
    }
}

#[test]
fn ui_handlers_reuse_the_store_owned_by_server_state() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let source = combined_sources(&source_root.join("ui"));
    assert!(source.contains("store: Store"));
    assert!(source.contains("scheduler: LocalSchedulerGateway"));
    assert!(!source.contains("Store::open(&state.paths.database)"));
}

#[test]
fn cli_and_ui_route_shared_flows_through_application_use_cases() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut cli = fs::read_to_string(source_root.join("cli.rs")).unwrap();
    cli.push_str(&combined_sources(&source_root.join("cli")));
    let mut ui = fs::read_to_string(source_root.join("ui.rs")).unwrap();
    ui.push_str(&combined_sources(&source_root.join("ui")));
    for operation in [
        "application::jobs::create_job",
        "application::jobs::query_jobs",
        "application::jobs::job_detail",
        "application::jobs::update_description",
        "application::jobs::commit_jobs",
        "application::jobs::cancel_job",
        "application::jobs::clean_jobs",
        "application::queue::queue_status",
        "application::queue::set_queue_locked",
        "application::queue::move_queued",
        "application::logs::read_logs",
        "application::configuration::configuration",
    ] {
        assert!(cli.contains(operation), "CLI bypasses {operation}");
        assert!(ui.contains(operation), "UI bypasses {operation}");
    }

    let submission = fs::read_to_string(source_root.join("submission.rs")).unwrap();
    assert!(submission.contains("jobs::create_job"));
    assert!(submission.contains("jobs::parse_command_line"));
    assert!(!submission.contains("while let Some(character)"));
}

#[test]
fn ui_stage_seven_keeps_axum_framework_and_io_at_adapter_edges() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let ui_root = source_root.join("ui");
    for file in [
        "assets.rs",
        "auth.rs",
        "dto.rs",
        "error.rs",
        "handlers/configuration.rs",
        "handlers/filesystem.rs",
        "handlers/jobs.rs",
        "handlers/queue.rs",
        "handlers/status.rs",
        "handlers/system.rs",
        "lifecycle.rs",
        "router.rs",
        "state.rs",
    ] {
        assert!(ui_root.join(file).is_file(), "missing UI module {file}");
    }

    let facade = fs::read_to_string(source_root.join("ui.rs")).unwrap();
    assert!(!facade.contains("axum::"));
    assert!(!facade.contains("TcpListener"));
    let router = production_source(&ui_root.join("router.rs"));
    assert!(router.contains("axum::Router"));
    assert!(!router.contains("TcpListener"));
    assert!(!router.contains("TcpStream"));
    assert_sources_avoid(
        &ui_root.join("handlers"),
        &[
            "Store::open",
            "ServiceClient::new",
            "TcpListener",
            "TcpStream",
        ],
    );

    for path in rust_sources(&ui_root) {
        let lines = production_source(&path).lines().count();
        assert!(
            lines <= 500,
            "{} has {lines} production lines; split it by adapter responsibility",
            path.display()
        );
    }
}

#[test]
fn browser_frontend_has_react_typescript_and_stylesheet_ownership() {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    let index = fs::read_to_string(web.join("index.html")).unwrap();
    assert!(index.contains("/src/main.tsx"));
    assert!(index.contains("<title>Stoker</title>"));
    assert!(index.contains("rel=\"icon\""));
    assert!(index.contains("/assets/logo-mark.png"));
    assert!(!web.join("app.js").exists());
    assert!(!web.join("modules").exists());
    assert!(web.join("dist/index.html").is_file());
    assert!(web.join("dist/app.js").is_file());
    assert!(web.join("dist/styles.css").is_file());
    for file in [
        "src/api.ts",
        "src/components.tsx",
        "src/context.tsx",
        "src/formatters.ts",
        "src/main.tsx",
        "src/pages/Configuration.tsx",
        "src/pages/Jobs.tsx",
        "src/pages/Logs.tsx",
        "src/pages/Overview.tsx",
        "src/pages/Queue.tsx",
        "src/state.ts",
        "src/types.ts",
        "styles/tokens.css",
        "styles/base.css",
        "styles/layout.css",
        "styles/components.css",
        "styles/overview.css",
        "styles/data-views.css",
        "styles/job-dialogs.css",
        "styles/responsive.css",
    ] {
        let path = web.join(file);
        assert!(path.is_file(), "missing frontend module {file}");
        assert!(
            fs::read_to_string(&path).unwrap().lines().count() <= 500,
            "frontend module {file} is too large"
        );
    }
}

#[test]
fn long_running_start_commands_use_the_shared_detachment_policy() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let scheduler = fs::read_to_string(source_root.join("cli/lifecycle/service.rs")).unwrap();
    let ui = fs::read_to_string(source_root.join("ui/lifecycle.rs")).unwrap();
    let process = fs::read_to_string(source_root.join("process/mod.rs")).unwrap();

    assert!(scheduler.contains("configure_detached(&mut command)"));
    assert!(ui.contains("configure_detached(&mut command)"));
    assert!(
        process.contains("command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)")
    );
    assert!(process.contains("nix::unistd::setsid()"));
}

#[test]
fn cli_stage_six_modules_keep_io_and_framework_dependencies_at_the_edges() {
    let cli_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");

    assert_source_avoids(
        &cli_root.join("args.rs"),
        &[
            "std::fs",
            "std::io",
            "std::process",
            "ServiceClient",
            "Store::open",
        ],
    );
    assert_sources_avoid(&cli_root.join("commands"), &["clap::"]);
    assert_sources_avoid(
        &cli_root.join("presentation"),
        &["std::fs", "std::process", "ServiceClient", "Store::open"],
    );
    assert_source_avoids(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/queue_editor/state.rs"),
        &["crossterm", "std::fs", "std::io", "std::process"],
    );
    assert_source_avoids(&cli_root.join("dispatch.rs"), &["set_var(", "remove_var("]);
    assert_sources_avoid(&cli_root.join("commands"), &["set_var(", "remove_var("]);

    for path in rust_sources(&cli_root) {
        let lines = fs::read_to_string(&path).unwrap().lines().count();
        assert!(
            lines <= 500,
            "{} has {lines} lines; split production modules before they become monoliths",
            path.display()
        );
    }
}

#[test]
fn application_component_tests_do_not_start_frameworks_or_external_services() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/application_use_cases.rs"),
    )
    .unwrap();
    for forbidden in [
        concat!("clap", "::"),
        concat!("Tcp", "Listener"),
        concat!("Tcp", "Stream"),
        concat!("Service", "::new"),
        concat!("Service", "Client"),
        concat!("Store", "::open"),
        concat!("brow", "ser"),
        concat!("#[tokio", "::test]"),
    ] {
        assert!(
            !source.contains(forbidden),
            "application component tests start forbidden dependency {forbidden:?}"
        );
    }
}
