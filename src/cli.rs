#[cfg(test)]
use std::process::Command;

#[cfg(test)]
use crate::StoreError;
#[cfg(test)]
use crate::config::{
    ConfigSnapshot, ConfigSnapshotEntry, ConfigSnapshotFile, ConfigSnapshotReason,
};
#[cfg(test)]
use crate::config::{ResolvedTimezone, resolve_timezone};
#[cfg(test)]
use crate::config::{StokerConfig, TimezoneSource};
#[cfg(test)]
use crate::domain::JobState;
#[cfg(test)]
use crate::domain::NewJob;
#[cfg(test)]
use crate::is_service_unavailable;
#[cfg(test)]
use crate::queue_editor::EditorMoveError;
#[cfg(test)]
use crate::submission::parse_command_line;
#[cfg(test)]
use crate::{StokerPaths, Store};

mod args;
mod extended;

#[cfg(test)]
use args::parse_job_state;
pub use args::{
    AddArgs, CancelArgs, Cli, CliCommand, ConfigCommand, ConfigKey, ConfirmationArgs, DbCommand,
    PolicyCommand, PolicyKey, QueueCommand, SetDescriptionArgs, UiCommand, UiStartArgs,
};

mod dispatch;

#[cfg(test)]
use dispatch::run_command_with_paths;
pub(crate) use dispatch::{open_store, runtime, scheduler_gateway};
pub use dispatch::{run, run_command};

mod presentation;

use presentation::messages::{application_cli_error, print_info, print_success, print_warning};

mod interactive;

#[cfg(test)]
use interactive::{
    InteractiveTerminalGuard, SnapshotSelectorAction, SnapshotSelectorState, SnapshotView,
    TimezoneSelectorAction, TimezoneSelectorState, detailed_snapshot_summary,
    format_snapshot_list_row, render_snapshot_selector, render_timezone_selector,
    snapshot_json_for_display, snapshot_list_header, snapshot_reason_width, snapshot_summary,
    summarize_keys,
};
use interactive::{restore_config, select_timezone};

mod lifecycle;

#[cfg(test)]
use lifecycle::service::terminate_child;

#[cfg(test)]
use commands::service::print_timezone_status;

mod commands;

#[cfg(test)]
use commands::jobs::commit;
#[cfg(test)]
use commands::queue::{editor_move_error, is_stale_move_error};

mod update;

#[cfg(all(test, unix))]
use update::remove_unix_binary;
use update::request_confirmation;
#[cfg(test)]
use update::{
    GithubAsset, GithubRelease, SystemUpdateGateway, UpdateGateway, checksum_for, download_bytes,
    download_release_binary_with_gateway, ensure_scheduler_stopped, is_confirmation,
    platform_binary_name, release_asset, update_with_gateway, windows_uninstall_script,
    windows_update_script,
};

#[cfg(test)]
use presentation::jobs::{format_jobs_row, format_jobs_row_for_terminal, format_optional_time};

#[cfg(test)]
fn commit_service_error(error: anyhow::Error) -> anyhow::Error {
    if is_service_unavailable(&error) {
        anyhow::anyhow!("Scheduler is not running. Run `stoker start` first.")
    } else {
        error
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::terminate_child;
    use std::process::Command;

    #[test]
    fn terminate_child_kills_and_waits_for_startup_failure_cleanup() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("spawn test child");
        terminate_child(&mut child);
        assert!(child.try_wait().expect("check test child").is_some());
    }
}

#[cfg(test)]
mod update_tests {
    use super::{
        Cli, CliCommand, GithubAsset, GithubRelease, SystemUpdateGateway, UpdateGateway,
        checksum_for, download_bytes, download_release_binary_with_gateway, is_confirmation,
        platform_binary_name, update_with_gateway,
    };
    use clap::{CommandFactory, Parser};
    use semver::Version;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::thread::JoinHandle;

    struct FakeUpdateGateway {
        tag_name: String,
        binary: Vec<u8>,
        checksums: Vec<u8>,
        latest_error: Option<String>,
        download_error: Option<String>,
        ensure_error: Option<String>,
        current_executable_error: Option<String>,
        install_error: Option<String>,
        confirmation: bool,
        calls: RefCell<Vec<String>>,
        installed: RefCell<Option<(PathBuf, Vec<u8>)>>,
    }

    impl FakeUpdateGateway {
        fn new(tag_name: &str) -> Self {
            let binary = b"fake release binary".to_vec();
            let digest = Sha256::digest(&binary)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Self {
                tag_name: tag_name.into(),
                checksums: format!("{digest}  {}\n", platform_binary_name().unwrap()).into_bytes(),
                binary,
                latest_error: None,
                download_error: None,
                ensure_error: None,
                current_executable_error: None,
                install_error: None,
                confirmation: false,
                calls: RefCell::new(Vec::new()),
                installed: RefCell::new(None),
            }
        }

        fn record(&self, operation: impl Into<String>) {
            self.calls.borrow_mut().push(operation.into());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl UpdateGateway for FakeUpdateGateway {
        fn latest_release(&self) -> anyhow::Result<GithubRelease> {
            self.record("latest_release");
            if let Some(error) = &self.latest_error {
                anyhow::bail!("{error}");
            }
            Ok(GithubRelease {
                tag_name: self.tag_name.clone(),
                assets: vec![
                    GithubAsset {
                        name: platform_binary_name()?.into(),
                        browser_download_url: "mock://binary".into(),
                    },
                    GithubAsset {
                        name: "SHA256SUMS".into(),
                        browser_download_url: "mock://checksums".into(),
                    },
                ],
            })
        }

        fn download_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
            self.record(format!("download {url}"));
            if let Some(error) = &self.download_error {
                anyhow::bail!("{error}");
            }
            match url {
                "mock://binary" => Ok(self.binary.clone()),
                "mock://checksums" => Ok(self.checksums.clone()),
                _ => anyhow::bail!("unexpected mock URL {url}"),
            }
        }

        fn ensure_scheduler_stopped(&self) -> anyhow::Result<()> {
            self.record("ensure_scheduler_stopped");
            if let Some(error) = &self.ensure_error {
                anyhow::bail!("{error}");
            }
            Ok(())
        }

        fn current_executable(&self) -> anyhow::Result<PathBuf> {
            self.record("current_executable");
            if let Some(error) = &self.current_executable_error {
                anyhow::bail!("{error}");
            }
            Ok(PathBuf::from("mock-current-executable"))
        }

        fn install_updated_binary(&self, current_exe: &Path, binary: &[u8]) -> anyhow::Result<()> {
            self.record("install_updated_binary");
            if let Some(error) = &self.install_error {
                anyhow::bail!("{error}");
            }
            *self.installed.borrow_mut() = Some((current_exe.to_path_buf(), binary.to_vec()));
            Ok(())
        }

        fn request_confirmation(&self, action: &str) -> anyhow::Result<bool> {
            self.record(format!("confirm {action}"));
            Ok(self.confirmation)
        }
    }

    fn serve_release_assets(
        binary: Vec<u8>,
        checksums: Vec<u8>,
    ) -> (String, String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let length = stream.read(&mut request).unwrap();
                let body = if String::from_utf8_lossy(&request[..length]).contains("/binary ") {
                    &binary
                } else {
                    &checksums
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        (
            format!("http://{address}/binary"),
            format!("http://{address}/checksums"),
            task,
        )
    }

    fn release_with_urls(binary_url: String, checksums_url: String) -> GithubRelease {
        GithubRelease {
            tag_name: "v1.2.3".into(),
            assets: vec![
                GithubAsset {
                    name: platform_binary_name().unwrap().into(),
                    browser_download_url: binary_url,
                },
                GithubAsset {
                    name: "SHA256SUMS".into(),
                    browser_download_url: checksums_url,
                },
            ],
        }
    }

    #[test]
    fn platform_binary_name_matches_release_assets() {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-windows-x86_64.exe");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-linux-x86_64");
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(platform_binary_name().unwrap(), "stoker-macos-arm64");
    }

    #[test]
    fn parses_checksum_for_the_exact_asset_name() {
        let checksums = "abc123  stoker-linux-x86_64\ndef456  stoker-linux-x86_64-extra";
        assert_eq!(
            checksum_for(checksums, "stoker-linux-x86_64").as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn update_confirmation_only_accepts_explicit_yes() {
        assert!(is_confirmation("y\n"));
        assert!(is_confirmation(" YES "));
        assert!(!is_confirmation(""));
        assert!(!is_confirmation("n"));
        assert!(!is_confirmation("anything else"));
    }

    #[test]
    fn release_binary_download_verifies_checksum_from_local_http_fixture() {
        let binary = b"test release binary".to_vec();
        let digest = Sha256::digest(&binary)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let checksums = format!("{digest}  {}\n", platform_binary_name().unwrap()).into_bytes();
        let (binary_url, checksums_url, server) = serve_release_assets(binary.clone(), checksums);
        let release = release_with_urls(binary_url, checksums_url);

        assert_eq!(
            download_release_binary_with_gateway(&release, &SystemUpdateGateway).unwrap(),
            binary
        );
        server.join().unwrap();
    }

    #[test]
    fn release_binary_download_rejects_bad_or_invalid_checksum_data() {
        let binary = b"test release binary".to_vec();
        let (binary_url, checksums_url, server) = serve_release_assets(
            binary.clone(),
            format!("0000  {}\n", platform_binary_name().unwrap()).into_bytes(),
        );
        let error = download_release_binary_with_gateway(
            &release_with_urls(binary_url, checksums_url),
            &SystemUpdateGateway,
        )
        .unwrap_err();
        assert!(error.to_string().contains("SHA-256 mismatch"));
        server.join().unwrap();

        let (binary_url, checksums_url, server) = serve_release_assets(binary, vec![0xff, 0xfe]);
        let error = download_release_binary_with_gateway(
            &release_with_urls(binary_url, checksums_url),
            &SystemUpdateGateway,
        )
        .unwrap_err();
        assert!(error.to_string().contains("decode SHA256SUMS"));
        server.join().unwrap();
    }

    fn release_tag_with_patch_delta(delta: i64) -> String {
        release_tag_from_version(Version::parse(env!("CARGO_PKG_VERSION")).unwrap(), delta)
    }

    fn release_tag_from_version(mut version: Version, delta: i64) -> String {
        if delta < 0 {
            if version.patch > 0 {
                version.patch -= 1;
            } else if version.minor > 0 {
                // Keep the older fixture valid when the package version is
                // released at a .0 patch boundary, such as v1.3.0.
                version.minor -= 1;
            } else {
                version.major = version.major.saturating_sub(1);
            }
        } else {
            version.patch += delta as u64;
        }
        format!("v{version}")
    }

    #[test]
    fn release_tag_patch_delta_handles_version_boundaries() {
        assert_eq!(
            release_tag_from_version(Version::new(1, 2, 3), -1),
            "v1.2.2"
        );
        assert_eq!(
            release_tag_from_version(Version::new(1, 2, 0), -1),
            "v1.1.0"
        );
        assert_eq!(
            release_tag_from_version(Version::new(0, 0, 0), -1),
            "v0.0.0"
        );
    }

    #[test]
    fn mock_update_gateway_covers_release_decisions_without_external_side_effects() {
        let older = release_tag_with_patch_delta(-1);
        let current = release_tag_with_patch_delta(0);
        let newer = release_tag_with_patch_delta(1);
        let mut gateway = FakeUpdateGateway::new(&older);
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("older than the installed"));
        assert_eq!(gateway.calls(), ["latest_release"]);

        gateway = FakeUpdateGateway::new(&current);
        update_with_gateway(true, &gateway).unwrap();
        assert_eq!(gateway.calls(), ["latest_release"]);

        gateway = FakeUpdateGateway::new("not-semver");
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("parse latest GitHub release version")
        );

        gateway = FakeUpdateGateway::new(&newer);
        gateway.confirmation = false;
        update_with_gateway(false, &gateway).unwrap();
        assert_eq!(
            gateway.calls(),
            ["latest_release", "confirm Continue with update"]
        );
    }

    #[test]
    fn mock_update_gateway_covers_update_side_effects_and_failures() {
        let newer = release_tag_with_patch_delta(1);
        let gateway = FakeUpdateGateway::new(&newer);
        update_with_gateway(true, &gateway).unwrap();
        assert_eq!(
            gateway.calls(),
            [
                "latest_release",
                "ensure_scheduler_stopped",
                "current_executable",
                "download mock://binary",
                "download mock://checksums",
                "install_updated_binary",
            ]
        );
        assert_eq!(
            gateway.installed.borrow().as_ref().unwrap().1,
            gateway.binary
        );

        let mut gateway = FakeUpdateGateway::new(&newer);
        gateway.ensure_error = Some("scheduler is still running".into());
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("scheduler is still running"));

        let mut gateway = FakeUpdateGateway::new(&newer);
        gateway.current_executable_error = Some("executable unavailable".into());
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("executable unavailable"));

        let mut gateway = FakeUpdateGateway::new(&newer);
        gateway.download_error = Some("download failed".into());
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("download failed"));

        let mut gateway = FakeUpdateGateway::new(&newer);
        gateway.install_error = Some("replacement failed".into());
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("replacement failed"));

        let mut gateway = FakeUpdateGateway::new(&newer);
        gateway.latest_error = Some("GitHub unavailable".into());
        let error = update_with_gateway(true, &gateway).unwrap_err();
        assert!(error.to_string().contains("GitHub unavailable"));

        let gateway = FakeUpdateGateway::new(&newer);
        assert!(gateway.download_bytes("mock://unexpected").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_binary_install_reports_replace_failure_and_cleans_up() {
        let directory = tempfile::tempdir().unwrap();

        let error =
            super::update::install_updated_binary(directory.path(), b"new binary").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("replace current Stoker executable")
        );
        assert!(directory.path().exists());
    }

    #[test]
    fn download_bytes_reports_http_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 128];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let error = download_bytes(&format!("http://{address}/missing")).unwrap_err();
        assert!(error.to_string().contains("download http://"));
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_uninstall_removes_only_the_requested_binary() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("stoker");
        std::fs::write(&executable, b"binary").unwrap();

        super::remove_unix_binary(&executable).unwrap();
        assert!(!executable.exists());
    }

    #[test]
    fn uninstall_is_a_valid_cli_command() {
        assert!(Cli::try_parse_from(["stoker", "uninstall"]).is_ok());
    }

    #[test]
    fn commit_accepts_multiple_selectors_and_rejects_mixed_modes() {
        let first = "00000000-0000-0000-0000-000000000001";
        let second = "00000000-0000-0000-0000-000000000002";
        let parsed = Cli::try_parse_from(["stoker", "commit", first, second]).unwrap();
        match parsed.command {
            CliCommand::Commit { ids, all, user } => {
                assert_eq!(ids.len(), 2);
                assert!(!all);
                assert_eq!(user, None);
            }
            command => panic!("unexpected command: {command:?}"),
        }

        assert!(Cli::try_parse_from(["stoker", "commit", "--all"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "commit", "--user", "alice"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "commit"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "commit", first, "--all"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "commit", "--all", "--user", "alice"]).is_err());

        let directory = tempfile::tempdir().unwrap();
        let paths = crate::StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        let error = super::commit(&paths, Vec::new(), false, Some("   ".into())).unwrap_err();
        assert_eq!(error.to_string(), "--user must not be empty");

        let error = super::commit_service_error(anyhow::anyhow!("queue is locked"));
        assert_eq!(error.to_string(), "queue is locked");
    }

    #[test]
    fn queue_operations_use_one_consistent_namespace() {
        for args in [
            ["stoker", "queue", "lock"],
            ["stoker", "queue", "edit"],
            ["stoker", "queue", "unlock"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn pause_and_resume_commands_are_removed() {
        assert!(Cli::try_parse_from(["stoker", "pause"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "resume"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "queue", "pause"]).is_err());
        assert!(Cli::try_parse_from(["stoker", "queue", "resume"]).is_err());
    }

    #[test]
    fn destructive_commands_accept_yes_flag() {
        for args in [
            vec!["stoker", "stop", "--yes"],
            vec!["stoker", "update", "--yes"],
            vec!["stoker", "uninstall", "--yes"],
            vec![
                "stoker",
                "cancel",
                "00000000-0000-0000-0000-000000000001",
                "--yes",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn timezone_aliases_are_accepted_for_all_commands() {
        assert!(Cli::try_parse_from(["stoker", "jobs", "--timezone", "Asia/Tokyo"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "stoker",
                "show",
                "00000000-0000-0000-0000-000000000001",
                "--tz",
                "UTC"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["stoker", "status", "--tz", "not/a-zone"]).is_ok());
    }

    #[test]
    fn config_restore_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "restore"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "config", "restore", "--tz", "UTC"]).is_ok());
    }

    #[test]
    fn config_snapshot_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "snapshot"]).is_ok());
    }

    #[test]
    fn config_show_is_a_valid_command() {
        assert!(Cli::try_parse_from(["stoker", "config", "show"]).is_ok());
    }

    #[test]
    fn description_commands_accept_set_and_clear_forms() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert!(Cli::try_parse_from(["stoker", "set-description", id, "Build"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "set-description", id, "--clear"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "set-description", id]).is_err());
        assert!(
            Cli::try_parse_from(["stoker", "set-description", id, "Build", "--clear"]).is_err()
        );
    }

    #[test]
    fn timezone_set_accepts_an_omitted_or_explicit_value() {
        assert!(Cli::try_parse_from(["stoker", "config", "set", "timezone"]).is_ok());
        assert!(Cli::try_parse_from(["stoker", "config", "set", "timezone", "Asia/Tokyo"]).is_ok());
    }

    #[test]
    fn top_level_help_describes_commands_and_timezone_option() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        for description in [
            "add",
            "Create a DRAFT job",
            "set-description",
            "Set or clear a job's description",
            "config",
            "Manage Stoker user configuration",
            "policy",
            "Manage scheduler execution and log policies",
            "status",
            "Show scheduler, queue, and timezone status",
            "queue",
            "Lock, edit, or unlock the queue",
            "      --timezone <TIMEZONE>  Timezone used when displaying timestamps [alias: --tz]",
        ] {
            assert!(
                help.contains(description),
                "missing help text: {description}"
            );
        }
    }
}

#[cfg(test)]
mod snapshot_selector_tests {
    use super::*;
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn valid_entry() -> ConfigSnapshotEntry {
        ConfigSnapshotEntry::Valid(ConfigSnapshotFile {
            path: PathBuf::from("config-test.json"),
            snapshot: ConfigSnapshot {
                snapshot_version: 1,
                created_at: Utc::now(),
                reason: crate::config::ConfigSnapshotReason::Initial,
                config: StokerConfig {
                    timezone: Some("Asia/Tokyo".to_owned()),
                },
            },
        })
    }

    #[test]
    fn enter_opens_details_and_escape_returns_to_list() {
        let entries = vec![valid_entry()];
        let mut state = SnapshotSelectorState::new();

        assert_eq!(
            state.reduce(key(KeyCode::Down), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::Enter), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::Detail);
        assert_eq!(
            state.reduce(key(KeyCode::Esc), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::List);
    }

    #[test]
    fn enter_then_yes_requests_restore_and_no_returns_to_list() {
        let entries = vec![valid_entry()];
        let mut state = SnapshotSelectorState::new();
        state.reduce(key(KeyCode::Enter), &entries);
        state.reduce(key(KeyCode::Enter), &entries);

        assert_eq!(state.view, SnapshotView::Confirm);
        assert_eq!(
            state.reduce(key(KeyCode::Char('x')), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::Char('n')), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::List);

        state.reduce(key(KeyCode::Enter), &entries);
        state.reduce(key(KeyCode::Enter), &entries);
        assert_eq!(
            state.reduce(key(KeyCode::Char('y')), &entries),
            SnapshotSelectorAction::Restore(0)
        );
    }

    #[test]
    fn invalid_snapshot_cannot_enter_confirmation() {
        let entries = vec![ConfigSnapshotEntry::Invalid {
            path: PathBuf::from("broken.json"),
            error: "invalid JSON".to_owned(),
        }];
        let mut state = SnapshotSelectorState::new();
        state.reduce(key(KeyCode::Enter), &entries);

        assert_eq!(state.view, SnapshotView::Detail);
        assert_eq!(
            state.reduce(key(KeyCode::Enter), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::Detail);
    }

    #[test]
    fn snapshot_list_header_columns_align_with_rows() {
        let reason_width = "manual".len();
        let header = snapshot_list_header(reason_width);
        let row = format_snapshot_list_row(
            '>',
            "2026-09-05T14:13:52.329+09:00",
            crate::config::ConfigSnapshotReason::Manual,
            "same as current",
            reason_width,
        );

        assert_eq!(header.find("Reason"), row.find("manual"));
        assert_eq!(header.find("Summary"), row.find("same as current"));
    }

    #[test]
    fn snapshot_json_includes_metadata_and_localizes_created_at() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        paths.ensure().expect("initialize temporary paths");

        let snapshot = match valid_entry() {
            ConfigSnapshotEntry::Valid(snapshot) => snapshot.snapshot,
            ConfigSnapshotEntry::Invalid { .. } => unreachable!(),
        };
        let timezone = resolve_timezone(&paths, Some("Asia/Tokyo")).expect("valid timezone");

        let json = snapshot_json_for_display(&snapshot, &timezone).expect("valid snapshot JSON");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert!(value.get("snapshot_version").is_some());
        assert_eq!(value["reason"], "initial");
        assert_eq!(value["config"]["timezone"], "Asia/Tokyo");
        assert!(value["created_at"].as_str().unwrap().ends_with("+09:00"));
    }

    #[test]
    fn timezone_selector_searches_and_selects_a_timezone() {
        let mut state = TimezoneSelectorState::new(
            vec![
                "UTC".to_owned(),
                "Asia/Taipei".to_owned(),
                "Asia/Tokyo".to_owned(),
            ],
            Some("Asia/Taipei"),
        );

        assert_eq!(state.selected_timezone(), Some("Asia/Taipei"));
        state.reduce(key(KeyCode::Char('t')));
        state.reduce(key(KeyCode::Char('o')));
        assert_eq!(state.selected_timezone(), Some("Asia/Tokyo"));
        assert_eq!(
            state.reduce(key(KeyCode::Enter)),
            TimezoneSelectorAction::Select("Asia/Tokyo".to_owned())
        );
    }

    #[test]
    fn timezone_selector_can_cancel_and_clear_search() {
        let mut state = TimezoneSelectorState::new(
            vec!["Asia/Taipei".to_owned(), "Asia/Tokyo".to_owned()],
            None,
        );

        state.reduce(key(KeyCode::Char('t')));
        state.reduce(key(KeyCode::Backspace));
        assert_eq!(state.query, "");
        assert_eq!(
            state.reduce(key(KeyCode::Char('q'))),
            TimezoneSelectorAction::Exit
        );

        let mut state = TimezoneSelectorState::new(
            vec!["Asia/Taipei".to_owned(), "Asia/Tokyo".to_owned()],
            None,
        );
        assert_eq!(
            state.reduce(key(KeyCode::Esc)),
            TimezoneSelectorAction::Exit
        );
    }
}

#[cfg(test)]
mod pure_logic_tests {
    use super::*;
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn paths(root: &std::path::Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    fn timezone() -> ResolvedTimezone {
        let directory = tempfile::tempdir().unwrap();
        resolve_timezone(&paths(directory.path()), Some("UTC")).unwrap()
    }

    fn valid_entry() -> ConfigSnapshotEntry {
        ConfigSnapshotEntry::Valid(ConfigSnapshotFile {
            path: PathBuf::from("config-test.json"),
            snapshot: ConfigSnapshot {
                snapshot_version: 1,
                created_at: Utc::now(),
                reason: ConfigSnapshotReason::Manual,
                config: StokerConfig {
                    timezone: Some("Asia/Tokyo".to_owned()),
                },
            },
        })
    }

    #[test]
    fn command_parser_handles_shell_quotes_escapes_and_and_operator() {
        assert_eq!(
            parse_command_line(r#"echo "hello world" && printf 'x y'"#).unwrap(),
            vec!["echo", "hello world", "&&", "printf", "x y"]
        );
        assert_eq!(
            parse_command_line("echo escaped\\ space \"quote: \\\"ok\\\"\"").unwrap(),
            vec!["echo", "escaped space", "quote: \"ok\""]
        );
        assert_eq!(
            parse_command_line("  ").unwrap_err().to_string(),
            "--cmd must not be empty"
        );
    }

    #[test]
    fn command_parser_reports_unterminated_quotes() {
        for input in ["echo 'missing", "echo \"missing"] {
            let error = parse_command_line(input).unwrap_err().to_string();
            assert!(error.contains("--cmd contains an unterminated"));
        }
    }

    #[test]
    fn command_parser_preserves_backslashes_that_do_not_escape_special_chars() {
        assert_eq!(
            parse_command_line(r#"C:\temp\file"#).unwrap(),
            vec![r#"C:\temp\file"#]
        );
        assert_eq!(parse_command_line(r#"'' """#).unwrap(), vec!["", ""]);
        assert_eq!(
            parse_command_line("echo foo&&bar").unwrap(),
            vec!["echo", "foo", "&&", "bar"]
        );
    }

    #[test]
    fn selector_reducers_cover_navigation_cancel_and_empty_results() {
        let mut timezone_state =
            TimezoneSelectorState::new(vec!["UTC".into(), "Asia/Tokyo".into()], None);
        assert_eq!(
            timezone_state.reduce(key(KeyCode::Down)),
            TimezoneSelectorAction::None
        );
        assert_eq!(timezone_state.selected, 1);
        assert_eq!(
            timezone_state.reduce(key(KeyCode::Up)),
            TimezoneSelectorAction::None
        );
        assert_eq!(timezone_state.selected, 0);
        assert_eq!(
            timezone_state.reduce(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            TimezoneSelectorAction::Exit
        );
        assert_eq!(
            timezone_state.reduce(key(KeyCode::F(1))),
            TimezoneSelectorAction::None
        );

        timezone_state.query = "does-not-exist".into();
        assert_eq!(
            timezone_state.reduce(key(KeyCode::Down)),
            TimezoneSelectorAction::None
        );
        assert_eq!(
            timezone_state.reduce(key(KeyCode::Enter)),
            TimezoneSelectorAction::None
        );
    }

    #[test]
    fn selector_reducers_cover_snapshot_navigation_and_confirmation_edges() {
        let entries = vec![valid_entry()];
        let mut state = SnapshotSelectorState::new();

        assert_eq!(
            state.reduce(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &entries
            ),
            SnapshotSelectorAction::Exit
        );
        assert_eq!(
            state.reduce(key(KeyCode::Up), &[]),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::Down), &[]),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::F(1)), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::Char('q')), &entries),
            SnapshotSelectorAction::Exit
        );

        state.reduce(key(KeyCode::Enter), &entries);
        assert_eq!(
            state.reduce(key(KeyCode::F(1)), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(
            state.reduce(key(KeyCode::Char('q')), &entries),
            SnapshotSelectorAction::None
        );
        assert_eq!(state.view, SnapshotView::List);

        state.reduce(key(KeyCode::Enter), &entries);
        state.reduce(key(KeyCode::Enter), &entries);
        assert_eq!(
            state.reduce(key(KeyCode::Char('Y')), &entries),
            SnapshotSelectorAction::Restore(0)
        );

        state.view = SnapshotView::Confirm;
        for code in [KeyCode::Char('N'), KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(
                state.reduce(key(code), &entries),
                SnapshotSelectorAction::None
            );
            assert_eq!(state.view, SnapshotView::List);
            state.view = SnapshotView::Confirm;
        }
    }

    #[test]
    fn timezone_selector_renderer_shows_empty_and_scrolled_results() {
        let timezones = (0..20).map(|index| format!("Etc/Zone{index:02}")).collect();
        let mut state = TimezoneSelectorState::new(timezones, None);
        state.selected = 19;
        let mut output = Vec::new();
        render_timezone_selector(&mut output, &state).unwrap();
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("Select timezone"));
        assert!(text.contains("> Etc/Zone19"));

        state.query = "missing".into();
        output.clear();
        render_timezone_selector(&mut output, &state).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("No matching IANA timezones."));
    }

    #[test]
    fn snapshot_renderer_covers_list_detail_and_confirmation_views() {
        let valid = valid_entry();
        let invalid = ConfigSnapshotEntry::Invalid {
            path: PathBuf::from("broken.json"),
            error: "invalid JSON".into(),
        };
        let current = StokerConfig {
            timezone: Some("UTC".into()),
        };
        let timezone = timezone();

        let mut state = SnapshotSelectorState::new();
        let entries = vec![valid.clone(), invalid.clone()];
        let mut output = Vec::new();
        render_snapshot_selector(&mut output, &state, &entries, &current, &timezone).unwrap();
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("Stoker configuration snapshots"));
        assert!(text.contains("invalid snapshot"));
        assert!(text.contains("1 changed: timezone"));

        state.selected = 0;
        state.view = SnapshotView::Detail;
        output.clear();
        render_snapshot_selector(&mut output, &state, &entries, &current, &timezone).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("Snapshot JSON:"));

        state.selected = 1;
        output.clear();
        render_snapshot_selector(&mut output, &state, &entries, &current, &timezone).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("cannot be restored"));

        state.selected = 0;
        state.view = SnapshotView::Confirm;
        output.clear();
        render_snapshot_selector(&mut output, &state, &entries, &current, &timezone).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("Continue?"));

        state.selected = 1;
        output.clear();
        render_snapshot_selector(&mut output, &state, &entries, &current, &timezone).unwrap();
        assert!(output.is_empty() || String::from_utf8_lossy(&output).contains("\u{1b}"));
    }

    #[test]
    fn snapshot_helpers_summarize_changes_and_long_key_lists() {
        let same = StokerConfig {
            timezone: Some("UTC".into()),
        };
        let changed = StokerConfig {
            timezone: Some("Asia/Tokyo".into()),
        };
        let snapshot = ConfigSnapshot {
            snapshot_version: 1,
            created_at: Utc::now(),
            reason: ConfigSnapshotReason::Manual,
            config: changed,
        };
        assert_eq!(snapshot_summary(&same, &snapshot), "1 changed: timezone");
        assert_eq!(detailed_snapshot_summary(&same, &snapshot), "  timezone");
        assert_eq!(
            snapshot_summary(
                &same,
                &ConfigSnapshot {
                    config: same.clone(),
                    ..snapshot.clone()
                }
            ),
            "same as current"
        );
        assert_eq!(
            detailed_snapshot_summary(
                &same,
                &ConfigSnapshot {
                    config: same.clone(),
                    ..snapshot
                }
            ),
            "No differences."
        );
        assert_eq!(snapshot_reason_width(&[]), "Reason".len());

        let keys = vec!["a-very-long-configuration-key".into(), "timezone".into()];
        let summary = summarize_keys(&keys);
        assert!(summary.ends_with("..."));
        assert!(summary.chars().count() <= 32);
    }

    #[test]
    fn stale_move_errors_are_classified_without_hiding_other_errors() {
        let id = uuid::Uuid::nil();
        for error in [
            anyhow::Error::new(StoreError::NotFound { id }),
            anyhow::Error::new(StoreError::InvalidQueueOrder {
                id,
                target_order: 2,
                queued_count: 1,
            }),
            anyhow::Error::new(StoreError::InvalidTransition {
                id,
                state: JobState::Queued,
                action: "move",
            }),
            anyhow::anyhow!("cannot move job {id} because it does not exist"),
        ] {
            assert!(is_stale_move_error(&error));
            assert!(matches!(editor_move_error(error), EditorMoveError::Stale));
        }
        let error = anyhow::anyhow!("unrelated failure");
        assert!(!is_stale_move_error(&error));
        assert!(matches!(
            editor_move_error(error),
            EditorMoveError::Callback(_)
        ));
    }

    #[test]
    fn release_helpers_validate_versions_and_assets() {
        let release = GithubRelease {
            tag_name: "v1.2.3".into(),
            assets: vec![GithubAsset {
                name: "stoker.exe".into(),
                browser_download_url: "https://example.test/stoker.exe".into(),
            }],
        };
        assert_eq!(release.version().unwrap(), semver::Version::new(1, 2, 3));
        assert_eq!(
            release_asset(&release, "stoker.exe").unwrap().name,
            "stoker.exe"
        );
        assert!(
            release_asset(&release, "missing")
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        assert!(
            GithubRelease {
                tag_name: "not-semver".into(),
                assets: Vec::new(),
            }
            .version()
            .is_err()
        );
    }

    #[test]
    fn parser_and_formatting_helpers_cover_common_boundaries() {
        assert_eq!(parse_job_state("running"), Ok(JobState::Running));
        assert!(parse_job_state("unknown").is_err());
        assert_eq!(
            format_jobs_row(
                ["id", "name", "state", "x", "y", "z", "w"],
                &[2, 4, 5, 1, 1, 1, 1]
            ),
            "id  name  state  x  y  z  w"
        );
        assert_eq!(
            format_jobs_row_for_terminal(
                ["id", "name", "x", "y", "SUCCEEDED", "z", "w"],
                &[2, 4, 1, 1, 9, 1, 1],
                false,
                false,
            ),
            "id  name  x  y  SUCCEEDED  z  w"
        );
        assert!(
            format_jobs_row_for_terminal(
                ["id", "name", "x", "y", "SUCCEEDED", "z", "w"],
                &[2, 4, 1, 1, 9, 1, 1],
                false,
                true,
            )
            .contains("\u{1b}[")
        );
        assert_eq!(format_optional_time(None, &timezone()), "-");
    }

    #[test]
    fn restore_without_snapshots_returns_without_entering_the_selector() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        restore_config(&paths, None).unwrap();
    }

    #[test]
    fn terminal_guard_can_restore_terminal_state() {
        let guard = InteractiveTerminalGuard;
        drop(guard);
    }
}

#[cfg(test)]
mod cli_runtime_tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct StokerHomeGuard {
        previous: Option<OsString>,
    }

    impl Drop for StokerHomeGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(value) => std::env::set_var("STOKER_HOME", value),
                    None => std::env::remove_var("STOKER_HOME"),
                }
            }
        }
    }

    fn use_home(path: &std::path::Path) -> StokerHomeGuard {
        let previous = std::env::var_os("STOKER_HOME");
        unsafe { std::env::set_var("STOKER_HOME", path) };
        StokerHomeGuard { previous }
    }

    #[test]
    fn command_dispatch_reaches_local_config_queue_and_job_flows() {
        let _lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let _home = use_home(directory.path());
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        paths.ensure().unwrap();
        let snapshot_dir = paths.snapshot_dir();
        if snapshot_dir.exists() {
            std::fs::remove_dir_all(snapshot_dir).unwrap();
        }

        let run_command = |command| run_command_with_paths(command, None, &paths);

        run_command(CliCommand::Config {
            command: ConfigCommand::Restore,
        })
        .unwrap();
        run_command(CliCommand::Config {
            command: ConfigCommand::Set {
                key: ConfigKey::Timezone,
                value: Some("UTC".into()),
            },
        })
        .unwrap();
        run_command(CliCommand::Config {
            command: ConfigCommand::Show,
        })
        .unwrap();
        run_command(CliCommand::Db {
            command: DbCommand::Check { integrity: false },
        })
        .unwrap();
        run_command(CliCommand::Config {
            command: ConfigCommand::Get {
                key: ConfigKey::Timezone,
            },
        })
        .unwrap();
        run_command(CliCommand::Config {
            command: ConfigCommand::Snapshot,
        })
        .unwrap();
        run_command(CliCommand::Config {
            command: ConfigCommand::Unset {
                key: ConfigKey::Timezone,
            },
        })
        .unwrap();

        run_command(CliCommand::Status).unwrap();
        ensure_scheduler_stopped().unwrap();
        assert!(
            run_command(CliCommand::Queue {
                command: QueueCommand::Edit,
            })
            .is_err()
        );
        run_command(CliCommand::Queue {
            command: QueueCommand::Lock,
        })
        .unwrap();
        run_command(CliCommand::Queue {
            command: QueueCommand::Edit,
        })
        .unwrap();
        run_command(CliCommand::Queue {
            command: QueueCommand::Unlock,
        })
        .unwrap();

        run_command(CliCommand::Add(AddArgs {
            user: "alice".into(),
            name: "local-flow".into(),
            description: Some("Created from add".into()),
            command: "echo hello".into(),
        }))
        .unwrap();
        assert!(
            run_command(CliCommand::Add(AddArgs {
                user: " ".into(),
                name: "invalid-user".into(),
                description: None,
                command: "echo hello".into(),
            }))
            .is_err()
        );
        assert!(
            run_command(CliCommand::Add(AddArgs {
                user: "alice".into(),
                name: " ".into(),
                description: None,
                command: "echo hello".into(),
            }))
            .is_err()
        );
        let store = Store::open(&paths.database).unwrap();
        let draft = store.list_jobs(None).unwrap().into_iter().next().unwrap();
        assert_eq!(draft.description.as_deref(), Some("Created from add"));
        run_command(CliCommand::SetDescription(SetDescriptionArgs {
            id: draft.id,
            description: Some("Created through the CLI".into()),
            clear: false,
        }))
        .unwrap();
        assert_eq!(
            Store::open(&paths.database)
                .unwrap()
                .get_job(draft.id)
                .unwrap()
                .description
                .as_deref(),
            Some("Created through the CLI")
        );
        run_command(CliCommand::SetDescription(SetDescriptionArgs {
            id: draft.id,
            description: None,
            clear: true,
        }))
        .unwrap();
        assert_eq!(
            Store::open(&paths.database)
                .unwrap()
                .get_job(draft.id)
                .unwrap()
                .description,
            None
        );
        run_command(CliCommand::Show { id: draft.id }).unwrap();
        run_command(CliCommand::Jobs {
            user: Some("alice".into()),
            state: Some(JobState::Draft),
        })
        .unwrap();
        let log_error = run_command(CliCommand::Logs {
            id: draft.id,
            follow: false,
        })
        .unwrap_err();
        assert!(log_error.to_string().contains("still DRAFT"));

        let commit_error = run_command(CliCommand::Commit {
            ids: vec![draft.id],
            all: false,
            user: None,
        })
        .unwrap_err();
        assert!(
            commit_error
                .to_string()
                .contains("Scheduler is not running")
        );

        let queued = store
            .create_job(NewJob {
                name: "queued".into(),
                user: "alice".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "queued".into()],
            })
            .unwrap();
        store.commit_job(queued).unwrap();
        let queued_log_error = run_command(CliCommand::Logs {
            id: queued,
            follow: false,
        })
        .unwrap_err();
        assert!(queued_log_error.to_string().contains("QUEUED"));
        let active = store.claim_next().unwrap().unwrap();
        run_command(CliCommand::Show { id: active.id }).unwrap();
        let active_log_error = run_command(CliCommand::Logs {
            id: active.id,
            follow: false,
        })
        .unwrap_err();
        assert!(
            active_log_error
                .to_string()
                .contains("No logs are available")
        );

        let finished = store
            .create_job(NewJob {
                name: "finished".into(),
                user: "alice".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "finished".into()],
            })
            .unwrap();
        store.commit_job(finished).unwrap();
        store.claim_next().unwrap();
        store.set_running(finished, 1).unwrap();
        store.finish(finished, Some(0), None).unwrap();
        let run_dir = paths.runs.join(finished.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("stdout.log"), b"out").unwrap();
        std::fs::write(run_dir.join("stderr.log"), b"err").unwrap();

        run_command(CliCommand::Show { id: finished }).unwrap();
        run_command(CliCommand::Logs {
            id: finished,
            follow: false,
        })
        .unwrap();
        run_command(CliCommand::Clean).unwrap();
        assert!(!run_dir.exists());
    }

    #[test]
    fn system_timezone_status_and_cross_platform_child_cleanup_are_reachable() {
        let _lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let _home = StokerHomeGuard {
            previous: std::env::var_os("STOKER_HOME"),
        };
        unsafe { std::env::remove_var("STOKER_HOME") };
        let fallback_paths = StokerPaths::from_env().unwrap();
        assert!(fallback_paths.root.ends_with(".stoker"));

        let directory = tempfile::tempdir().unwrap();
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        let timezone = resolve_timezone(&paths, None).unwrap();
        assert_eq!(timezone.source, TimezoneSource::System);
        print_timezone_status(&paths, &timezone);

        #[cfg(unix)]
        let mut child = Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap();
        #[cfg(windows)]
        let mut child = Command::new("cmd")
            .args(["/C", "ping 127.0.0.1 -n 31 > NUL"])
            .spawn()
            .unwrap();
        terminate_child(&mut child);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn home_guard_restores_a_previously_set_home() {
        let _lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous = std::env::var_os("STOKER_HOME");
        unsafe { std::env::set_var("STOKER_HOME", "previous-home") };
        {
            let _guard = use_home(std::path::Path::new("replacement-home"));
        }
        assert_eq!(
            std::env::var_os("STOKER_HOME"),
            Some(OsString::from("previous-home"))
        );
        drop(StokerHomeGuard { previous });
    }
}

#[cfg(test)]
mod windows_uninstall_tests {
    use super::{windows_uninstall_script, windows_update_script};
    use std::path::Path;

    #[test]
    fn uninstall_helper_waits_for_stoker_then_removes_the_binary() {
        let script = windows_uninstall_script(1234, Path::new(r"C:\Tools\stoker.exe"));
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("del /F /Q \"C:\\Tools\\stoker.exe\""));
        assert!(script.contains("start \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1"));
        assert!(script.ends_with("exit /B 0\r\n"));
    }

    #[test]
    fn update_helper_waits_for_stoker_then_moves_the_downloaded_binary() {
        let script = windows_update_script(
            1234,
            Path::new(r"C:\Tools\stoker.exe"),
            Path::new(r"C:\Temp\stoker.exe"),
            Path::new(r"C:\Temp\stoker-update"),
        );
        assert!(script.contains("PID eq 1234"));
        assert!(script.contains("move /Y \"C:\\Temp\\stoker.exe\" \"C:\\Tools\\stoker.exe\""));
        assert!(script.contains("Stoker update completed successfully."));
        assert!(script.contains("Stoker update failed: could not replace the executable."));
        assert!(script.contains("start \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1"));
        assert!(script.ends_with("exit /B 0\r\n"));
    }
}
