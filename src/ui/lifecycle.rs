use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use crossterm::style::Color;
use tokio::net::TcpListener;

use crate::Store;
use crate::config::StokerPaths;
use crate::ipc::ServiceClient;
use crate::output;
use crate::process::spawn_detached;

use super::dto::UiMetadata;
use super::router::build_router;
use super::state::ApiState;

pub(super) const DEFAULT_PORT: u16 = 8765;

/// Start a detached UI process and wait until its listener and metadata agree.
pub fn start(paths: StokerPaths, host: IpAddr, port: u16, open: bool) -> anyhow::Result<()> {
    paths.ensure()?;
    if let Some(metadata) = read_metadata(&paths)?
        && probe_host(connect_host(metadata.host), metadata.port).is_ok()
    {
        print_notice(format!(
            "Stoker UI is already running at {}",
            ui_url(&metadata)
        ));
        return Ok(());
    }

    remove_if_exists(&paths.ui_metadata())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.ui_log())
        .context("open Stoker UI log")?;
    let log_err = log.try_clone().context("duplicate Stoker UI log")?;
    let executable = std::env::current_exe().context("locate stoker executable")?;
    let mut command = std::process::Command::new(executable);
    command
        .args([
            "ui-run",
            "--host",
            &host.to_string(),
            "--port",
            &port.to_string(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    let child = spawn_detached(&mut command).context("start Stoker UI server")?;

    let mut gateway = SystemUiStartupGateway {
        paths,
        host: connect_host(host),
        port,
        child,
        started: std::time::Instant::now(),
        timeout: Duration::from_secs(5),
    };
    let metadata = wait_for_ui_start(&mut gateway)?;
    let url = ui_url(&metadata);
    print_start_message(&metadata, &url, open)
}

pub fn status(paths: StokerPaths) -> anyhow::Result<()> {
    paths.ensure()?;
    let Some(metadata) = read_metadata(&paths)? else {
        println!("Stoker UI: stopped");
        return Ok(());
    };
    if probe_host(connect_host(metadata.host), metadata.port).is_err() {
        println!("Stoker UI: stopped (stale metadata found)");
        return Ok(());
    }
    println!("Stoker UI: running");
    println!("PID: {}", metadata.pid);
    println!("URL: {}", ui_url(&metadata));
    println!("Bind: {}", metadata.host);
    Ok(())
}

pub fn stop(paths: StokerPaths) -> anyhow::Result<()> {
    paths.ensure()?;
    let Some(metadata) = read_metadata(&paths)? else {
        println!("Stoker UI is not running.");
        return Ok(());
    };
    if probe_host(connect_host(metadata.host), metadata.port).is_err() {
        remove_if_exists(&paths.ui_metadata())?;
        print_notice("Stoker UI was not reachable; removed stale metadata.");
        return Ok(());
    }
    let request = "POST /__stoker/shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_owned();
    let response = request_blocking(
        connect_host(metadata.host),
        metadata.port,
        request.as_bytes(),
    )?;
    if !response.starts_with(b"HTTP/1.1 200") {
        anyhow::bail!("UI server rejected stop request")
    }
    let mut gateway = SystemUiStopGateway {
        paths,
        host: connect_host(metadata.host),
        port: metadata.port,
        started: std::time::Instant::now(),
        timeout: Duration::from_secs(5),
    };
    wait_for_ui_stop(&mut gateway)?;
    print_success("Stoker UI stopped.");
    Ok(())
}

pub fn run(paths: StokerPaths, host: IpAddr, port: u16) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("create UI runtime")?;
    runtime.block_on(run_async(paths, host, port))
}

pub(super) async fn run_async(paths: StokerPaths, host: IpAddr, port: u16) -> anyhow::Result<()> {
    paths.ensure()?;
    let store = Store::open(&paths.database).context("open UI database")?;
    let listener = TcpListener::bind(SocketAddr::new(host, port))
        .await
        .with_context(|| format!("bind Stoker UI at {host}:{port}"))?;
    let actual = listener.local_addr().context("read Stoker UI address")?;
    let metadata = UiMetadata {
        pid: std::process::id(),
        host,
        port: actual.port(),
    };
    write_metadata(&paths, &metadata)?;
    let state = ApiState::new(paths.clone(), store, ServiceClient::new(paths.clone()));
    let shutdown = state.shutdown.clone();
    let result = axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await
        .context("serve Stoker UI");
    let cleanup = remove_if_exists(&paths.ui_metadata());
    result.and(cleanup)
}

pub(super) fn read_metadata(paths: &StokerPaths) -> anyhow::Result<Option<UiMetadata>> {
    let path = paths.ui_metadata();
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read UI metadata {}", path.display()))?;
    Ok(Some(
        serde_json::from_str(&contents).context("parse UI metadata")?,
    ))
}

fn write_metadata(paths: &StokerPaths, metadata: &UiMetadata) -> anyhow::Result<()> {
    let path = paths.ui_metadata();
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_string_pretty(metadata)? + "\n";
    fs::write(&temporary, contents)
        .with_context(|| format!("write UI metadata {}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .with_context(|| format!("install UI metadata {}", path.display()))?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

fn connect_host(host: IpAddr) -> IpAddr {
    if !host.is_unspecified() {
        return host;
    }
    match host {
        IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
    }
}

fn ui_url(metadata: &UiMetadata) -> String {
    let host = connect_host(metadata.host);
    let formatted_host = match host {
        IpAddr::V4(host) => host.to_string(),
        IpAddr::V6(host) => format!("[{host}]"),
    };
    format!("http://{formatted_host}:{}", metadata.port)
}

fn probe_host(host: IpAddr, port: u16) -> std::io::Result<()> {
    std::net::TcpStream::connect_timeout(&SocketAddr::new(host, port), Duration::from_millis(100))
        .map(|_| ())
}

fn request_blocking(host: IpAddr, port: u16, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut stream =
        std::net::TcpStream::connect_timeout(&SocketAddr::new(host, port), Duration::from_secs(2))
            .context("connect to Stoker UI")?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(payload)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(response)
}

fn terminate_child(child: &mut std::process::Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

trait UiStartupGateway {
    fn reachable(&mut self) -> bool;
    fn metadata(&mut self) -> anyhow::Result<Option<UiMetadata>>;
    fn child_exit(&mut self) -> anyhow::Result<Option<String>>;
    fn timed_out(&self) -> bool;
    fn pause(&mut self, duration: Duration);
    fn cleanup(&mut self) -> anyhow::Result<()>;
}

struct SystemUiStartupGateway {
    paths: StokerPaths,
    host: IpAddr,
    port: u16,
    child: std::process::Child,
    started: std::time::Instant,
    timeout: Duration,
}

impl UiStartupGateway for SystemUiStartupGateway {
    fn reachable(&mut self) -> bool {
        probe_host(self.host, self.port).is_ok()
    }

    fn metadata(&mut self) -> anyhow::Result<Option<UiMetadata>> {
        read_metadata(&self.paths)
    }

    fn child_exit(&mut self) -> anyhow::Result<Option<String>> {
        self.child
            .try_wait()
            .context("check Stoker UI server")
            .map(|status| status.map(|status| status.to_string()))
    }

    fn timed_out(&self) -> bool {
        self.started.elapsed() >= self.timeout
    }

    fn pause(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn cleanup(&mut self) -> anyhow::Result<()> {
        terminate_child(&mut self.child);
        remove_if_exists(&self.paths.ui_metadata())
    }
}

fn wait_for_ui_start<G: UiStartupGateway>(gateway: &mut G) -> anyhow::Result<UiMetadata> {
    loop {
        if gateway.reachable() {
            match gateway.metadata() {
                Ok(Some(metadata)) => return Ok(metadata),
                Ok(None) => {
                    let error =
                        anyhow::anyhow!("UI listener is reachable but metadata was not written");
                    let _ = gateway.cleanup();
                    return Err(error);
                }
                Err(error) => {
                    let _ = gateway.cleanup();
                    return Err(error);
                }
            }
        }
        match gateway.child_exit() {
            Ok(Some(status)) => {
                let _ = gateway.cleanup();
                anyhow::bail!("Stoker UI server exited during startup ({status})");
            }
            Ok(None) => {}
            Err(error) => {
                let _ = gateway.cleanup();
                return Err(error);
            }
        }
        if gateway.timed_out() {
            let _ = gateway.cleanup();
            anyhow::bail!("timed out waiting for Stoker UI server to start");
        }
        gateway.pause(Duration::from_millis(50));
    }
}

trait UiStopGateway {
    fn stopped(&mut self) -> bool;
    fn timed_out(&self) -> bool;
    fn pause(&mut self, duration: Duration);
}

struct SystemUiStopGateway {
    paths: StokerPaths,
    host: IpAddr,
    port: u16,
    started: std::time::Instant,
    timeout: Duration,
}

impl UiStopGateway for SystemUiStopGateway {
    fn stopped(&mut self) -> bool {
        probe_host(self.host, self.port).is_err() && !self.paths.ui_metadata().exists()
    }

    fn timed_out(&self) -> bool {
        self.started.elapsed() >= self.timeout
    }

    fn pause(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn wait_for_ui_stop<G: UiStopGateway>(gateway: &mut G) -> anyhow::Result<()> {
    loop {
        if gateway.stopped() {
            return Ok(());
        }
        if gateway.timed_out() {
            anyhow::bail!("UI server did not stop within 5 seconds");
        }
        gateway.pause(Duration::from_millis(50));
    }
}

fn print_start_message(metadata: &UiMetadata, url: &str, open: bool) -> anyhow::Result<()> {
    print_success("Stoker UI started.");
    println!("URL: {url}");
    println!("PID: {}", metadata.pid);
    if open {
        open_browser(url)?;
    }
    Ok(())
}

fn print_success(message: impl std::fmt::Display) {
    println!(
        "{}",
        output::paint(message, Color::Green, output::stdout_color_enabled())
    );
}

fn print_notice(message: impl std::fmt::Display) {
    println!(
        "{}",
        output::paint(message, Color::Yellow, output::stdout_color_enabled())
    );
}

fn open_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .context("open Stoker UI in the default browser")?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .context("open Stoker UI in the default browser")?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .context("open Stoker UI in the default browser")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::NewJob;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn filesystem_roots_include_a_recent_existing_job_directory() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(&directory.path().join("data"));
        paths.ensure().unwrap();
        let recent = directory.path().join("recent-work");
        fs::create_dir(&recent).unwrap();
        let store = Store::open(&paths.database).unwrap();
        store
            .create_job(NewJob {
                name: "recent".into(),
                user: "tester".into(),
                description: None,
                cwd: recent,
                command: vec!["echo".into(), "recent".into()],
            })
            .unwrap();
        let state = ApiState::new(paths.clone(), store, ServiceClient::new(paths));
        let response = crate::ui::handlers::filesystem::roots(axum::extract::State(state))
            .await
            .unwrap()
            .0;
        assert!(response.default_path.is_some());
        assert!(
            response
                .locations
                .iter()
                .any(|location| location.kind == "recent"
                    && location.label.contains("recent-work")
                    && location.path.ends_with("recent-work"))
        );
    }

    struct FakeUiStartupGateway {
        reachable: VecDeque<bool>,
        metadata: Option<UiMetadata>,
        metadata_error: Option<String>,
        child_exit: Option<String>,
        child_error: Option<String>,
        timed_out: bool,
        pauses: Vec<Duration>,
        cleanup_calls: usize,
    }

    impl FakeUiStartupGateway {
        fn waiting() -> Self {
            Self {
                reachable: VecDeque::from([false]),
                metadata: Some(UiMetadata {
                    pid: 42,
                    host: "127.0.0.1".parse().unwrap(),
                    port: 8765,
                }),
                metadata_error: None,
                child_exit: None,
                child_error: None,
                timed_out: false,
                pauses: Vec::new(),
                cleanup_calls: 0,
            }
        }
    }

    impl UiStartupGateway for FakeUiStartupGateway {
        fn reachable(&mut self) -> bool {
            self.reachable.pop_front().unwrap_or(true)
        }
        fn metadata(&mut self) -> anyhow::Result<Option<UiMetadata>> {
            if let Some(error) = self.metadata_error.take() {
                anyhow::bail!(error);
            }
            Ok(self.metadata.take())
        }
        fn child_exit(&mut self) -> anyhow::Result<Option<String>> {
            if let Some(error) = self.child_error.take() {
                anyhow::bail!(error);
            }
            Ok(self.child_exit.take())
        }
        fn timed_out(&self) -> bool {
            self.timed_out
        }
        fn pause(&mut self, duration: Duration) {
            self.pauses.push(duration);
        }
        fn cleanup(&mut self) -> anyhow::Result<()> {
            self.cleanup_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn startup_retry_timeout_and_failures_use_injected_seams() {
        let mut success = FakeUiStartupGateway::waiting();
        assert_eq!(wait_for_ui_start(&mut success).unwrap().pid, 42);
        assert_eq!(success.pauses, vec![Duration::from_millis(50)]);

        let mut timeout = FakeUiStartupGateway::waiting();
        timeout.timed_out = true;
        assert!(
            wait_for_ui_start(&mut timeout)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert_eq!(timeout.cleanup_calls, 1);

        let mut exited = FakeUiStartupGateway::waiting();
        exited.child_exit = Some("exit code: 9".into());
        assert!(
            wait_for_ui_start(&mut exited)
                .unwrap_err()
                .to_string()
                .contains("exit code: 9")
        );
        assert_eq!(exited.cleanup_calls, 1);

        let mut missing = FakeUiStartupGateway::waiting();
        missing.reachable = VecDeque::from([true]);
        missing.metadata = None;
        assert!(
            wait_for_ui_start(&mut missing)
                .unwrap_err()
                .to_string()
                .contains("metadata")
        );
        assert_eq!(missing.cleanup_calls, 1);

        let mut metadata_error = FakeUiStartupGateway::waiting();
        metadata_error.reachable = VecDeque::from([true]);
        metadata_error.metadata_error = Some("metadata failed".into());
        assert_eq!(
            wait_for_ui_start(&mut metadata_error)
                .unwrap_err()
                .to_string(),
            "metadata failed"
        );
        assert_eq!(metadata_error.cleanup_calls, 1);

        let mut child_error = FakeUiStartupGateway::waiting();
        child_error.child_error = Some("child probe failed".into());
        assert_eq!(
            wait_for_ui_start(&mut child_error).unwrap_err().to_string(),
            "child probe failed"
        );
        assert_eq!(child_error.cleanup_calls, 1);
    }

    struct FakeStop {
        stopped: VecDeque<bool>,
        timed_out: bool,
        pauses: usize,
    }

    impl UiStopGateway for FakeStop {
        fn stopped(&mut self) -> bool {
            self.stopped.pop_front().unwrap_or(true)
        }
        fn timed_out(&self) -> bool {
            self.timed_out
        }
        fn pause(&mut self, _duration: Duration) {
            self.pauses += 1;
        }
    }

    #[test]
    fn stop_retry_and_timeout_are_deterministic() {
        let mut success = FakeStop {
            stopped: VecDeque::from([false, true]),
            timed_out: false,
            pauses: 0,
        };
        wait_for_ui_stop(&mut success).unwrap();
        assert_eq!(success.pauses, 1);
        let mut timeout = FakeStop {
            stopped: VecDeque::from([false]),
            timed_out: true,
            pauses: 0,
        };
        assert!(
            wait_for_ui_stop(&mut timeout)
                .unwrap_err()
                .to_string()
                .contains("5 seconds")
        );
    }

    #[test]
    fn metadata_and_urls_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        assert!(read_metadata(&paths).unwrap().is_none());
        let metadata = UiMetadata {
            pid: 42,
            host: "::".parse().unwrap(),
            port: 9000,
        };
        write_metadata(&paths, &metadata).unwrap();
        assert_eq!(read_metadata(&paths).unwrap(), Some(metadata.clone()));
        assert_eq!(ui_url(&metadata), "http://[::1]:9000");
        assert_eq!(
            connect_host("0.0.0.0".parse().unwrap()),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            connect_host("127.0.0.1".parse().unwrap()),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        remove_if_exists(&paths.ui_metadata()).unwrap();
        remove_if_exists(&paths.ui_metadata()).unwrap();
    }

    #[test]
    fn public_lifecycle_handles_stopped_running_and_stale_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        status(paths.clone()).unwrap();
        stop(paths.clone()).unwrap();

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        write_metadata(
            &paths,
            &UiMetadata {
                pid: 99,
                host: "127.0.0.1".parse().unwrap(),
                port,
            },
        )
        .unwrap();
        start(paths.clone(), "127.0.0.1".parse().unwrap(), port, false).unwrap();
        status(paths.clone()).unwrap();
        drop(listener);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while probe_host("127.0.0.1".parse().unwrap(), port).is_ok() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        status(paths.clone()).unwrap();
        stop(paths.clone()).unwrap();
        assert!(read_metadata(&paths).unwrap().is_none());

        print_start_message(
            &UiMetadata {
                pid: 1,
                host: "127.0.0.1".parse().unwrap(),
                port: 8765,
            },
            "http://127.0.0.1:8765",
            false,
        )
        .unwrap();
        print_start_message(
            &UiMetadata {
                pid: 2,
                host: "0.0.0.0".parse().unwrap(),
                port: 8765,
            },
            "http://127.0.0.1:8765",
            false,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn axum_server_serves_embedded_assets_and_shutdown_cleans_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let server_paths = paths.clone();
        let server = tokio::spawn(async move {
            run_async(server_paths, "127.0.0.1".parse().unwrap(), 0)
                .await
                .unwrap();
        });
        let metadata = wait_for_metadata(&paths).await;
        let bundle = raw_request(
            metadata.port,
            "GET /app.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(bundle.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&bundle).contains("createRoot"));

        let shutdown = raw_request(
            metadata.port,
            "POST /__stoker/shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(shutdown.starts_with(b"HTTP/1.1 200 OK"));
        server.await.unwrap();
        assert!(read_metadata(&paths).unwrap().is_none());
    }

    #[tokio::test]
    async fn public_stop_requests_graceful_shutdown_and_waits_for_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let server_paths = paths.clone();
        let server = tokio::spawn(async move {
            run_async(server_paths, "127.0.0.1".parse().unwrap(), 0)
                .await
                .unwrap();
        });
        wait_for_metadata(&paths).await;
        let stop_paths = paths.clone();
        tokio::task::spawn_blocking(move || stop(stop_paths))
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert!(read_metadata(&paths).unwrap().is_none());
    }

    async fn wait_for_metadata(paths: &StokerPaths) -> UiMetadata {
        for _ in 0..100 {
            if let Some(metadata) = read_metadata(paths).unwrap() {
                return metadata;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("UI metadata was not written");
    }

    async fn raw_request(port: u16, request: &str) -> Vec<u8> {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    }

    fn test_paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }
}
