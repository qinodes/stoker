//! Embedded browser UI server and its typed HTTP control API.
//!
//! Embedded assets and typed browser operations backed by the existing Store
//! and scheduler IPC. The browser never owns a second job or queue model.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use crossterm::style::Color;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::config::{ResolvedTimezone, StokerPaths, TimezoneSource, resolve_timezone};
use crate::domain::{
    Job, JobState, MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH, MAX_JOB_USER_LENGTH,
};
use crate::ipc::{ServiceClient, is_service_unavailable};
use crate::output;
use crate::{Store, StoreError};

const DEFAULT_PORT: u16 = 8765;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_LOG_BYTES: usize = 256 * 1024;
const UI_TOKEN_ENV: &str = "STOKER_UI_TOKEN";

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLES_CSS: &str = include_str!("../web/styles.css");
const APP_JS: &str = include_str!("../web/app.js");
const LOGO_SVG: &str = include_str!("../assets/logo.svg");
const LOGO_MARK_PNG: &[u8] = include_bytes!("../assets/logo-mark.png");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMetadata {
    pub pid: u32,
    pub host: IpAddr,
    pub port: u16,
    pub auth_required: bool,
}

#[derive(Debug, Clone, Copy)]
struct HttpStatus {
    code: u16,
    reason: &'static str,
}

impl HttpStatus {
    const OK: Self = Self {
        code: 200,
        reason: "OK",
    };
    const CREATED: Self = Self {
        code: 201,
        reason: "Created",
    };
    const BAD_REQUEST: Self = Self {
        code: 400,
        reason: "Bad Request",
    };
    const UNAUTHORIZED: Self = Self {
        code: 401,
        reason: "Unauthorized",
    };
    const FORBIDDEN: Self = Self {
        code: 403,
        reason: "Forbidden",
    };
    const CONFLICT: Self = Self {
        code: 409,
        reason: "Conflict",
    };
    const NOT_FOUND: Self = Self {
        code: 404,
        reason: "Not Found",
    };
    const METHOD_NOT_ALLOWED: Self = Self {
        code: 405,
        reason: "Method Not Allowed",
    };
    const PAYLOAD_TOO_LARGE: Self = Self {
        code: 413,
        reason: "Payload Too Large",
    };
    const UNSUPPORTED_MEDIA_TYPE: Self = Self {
        code: 415,
        reason: "Unsupported Media Type",
    };
    const INTERNAL_SERVER_ERROR: Self = Self {
        code: 500,
        reason: "Internal Server Error",
    };
    const SERVICE_UNAVAILABLE: Self = Self {
        code: 503,
        reason: "Service Unavailable",
    };
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct UiServerState {
    paths: StokerPaths,
    metadata: UiMetadata,
    token: Option<String>,
    shutdown: Arc<Notify>,
    stopping: Arc<AtomicBool>,
}

#[derive(Debug, Serialize)]
struct UiConfigResponse {
    auth_required: bool,
    version: &'static str,
    max_job_name_length: usize,
    max_job_user_length: usize,
    max_job_description_length: usize,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    scheduler: SchedulerResponse,
    counts: CountResponse,
    queue_locked: bool,
    timezone: TimezoneResponse,
    generated_at: String,
}

#[derive(Debug, Serialize)]
struct SchedulerResponse {
    running: bool,
    pid: Option<u32>,
    active_job: Option<Uuid>,
    queued_jobs: usize,
}

#[derive(Debug, Serialize)]
struct CountResponse {
    total: usize,
    draft: usize,
    queued: usize,
    active: usize,
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, Serialize)]
struct TimezoneResponse {
    name: String,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct JobsResponse {
    jobs: Vec<Job>,
    timezone: TimezoneResponse,
}

#[derive(Debug, Serialize)]
struct QueueResponse {
    jobs: Vec<Job>,
    locked: bool,
}

#[derive(Debug, Serialize)]
struct CreateJobResponse {
    job: Job,
}

#[derive(Debug, Serialize)]
struct JobDetailResponse {
    job: Job,
    working_directory_status: &'static str,
    display_timezone: String,
}

#[derive(Debug, Serialize)]
struct JobActionResponse {
    job: Job,
}

#[derive(Debug, Deserialize)]
struct CreateJobRequest {
    user: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    cwd: String,
    command: String,
}

#[derive(Debug, Deserialize)]
struct UpdateDescriptionRequest {
    description: Option<String>,
    expected_revision: i64,
}

#[derive(Debug, Serialize)]
struct FsLocation {
    kind: &'static str,
    label: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct FsRootsResponse {
    default_path: Option<String>,
    locations: Vec<FsLocation>,
}

#[derive(Debug, Serialize)]
struct FsDirectory {
    name: String,
    path: String,
    is_symlink: bool,
}

#[derive(Debug, Serialize)]
struct FsDirectoriesResponse {
    path: String,
    parent: Option<String>,
    directories: Vec<FsDirectory>,
    truncated: bool,
    skipped_entries: usize,
}

#[derive(Debug, Serialize)]
struct CleanResponse {
    removed: usize,
}

#[derive(Debug, Serialize)]
struct LogsResponse {
    job: Job,
    stdout: String,
    stderr: String,
    stdout_available: bool,
    stderr_available: bool,
    stdout_truncated: bool,
    stderr_truncated: bool,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConfigurationResponse {
    config: crate::config::StokerConfig,
    effective_timezone: TimezoneResponse,
    timezones: Vec<String>,
    config_path: String,
    snapshot_dir: String,
    snapshots: Vec<SnapshotResponse>,
}

#[derive(Debug, Serialize)]
struct SnapshotResponse {
    path: String,
    valid: bool,
    created_at: Option<String>,
    reason: Option<String>,
    timezone: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QueueMoveRequest {
    target_order: usize,
}

#[derive(Debug, Deserialize)]
struct TimezoneRequest {
    value: String,
}

#[derive(Debug, Deserialize)]
struct RestoreRequest {
    path: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn default_port() -> u16 {
    DEFAULT_PORT
}

/// Start a detached UI process and wait until its TCP listener is reachable.
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
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    write_token(&paths, &token)?;

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.ui_log())
        .context("open Stoker UI log")?;
    let log_err = log.try_clone().context("duplicate Stoker UI log")?;
    let executable = std::env::current_exe().context("locate stoker executable")?;
    let mut child = std::process::Command::new(executable)
        .args([
            "ui-run",
            "--host",
            &host.to_string(),
            "--port",
            &port.to_string(),
        ])
        .env(UI_TOKEN_ENV, &token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err))
        .spawn()
        .context("start Stoker UI server")?;

    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if probe_host(connect_host(host), port).is_ok() {
            let metadata = read_metadata(&paths)?.ok_or_else(|| {
                anyhow::anyhow!("UI listener is reachable but metadata was not written")
            })?;
            let url = ui_url(&metadata);
            print_start_message(&metadata, &url, &token, open)?;
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("check Stoker UI server")? {
            remove_if_exists(&paths.ui_metadata())?;
            anyhow::bail!("Stoker UI server exited during startup ({status})");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    terminate_child(&mut child);
    remove_if_exists(&paths.ui_metadata())?;
    anyhow::bail!("timed out waiting for Stoker UI server to start")
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
    println!(
        "Authentication: {}",
        if metadata.auth_required {
            "token required"
        } else {
            "local only"
        }
    );
    Ok(())
}

pub fn stop(paths: StokerPaths) -> anyhow::Result<()> {
    paths.ensure()?;
    let Some(metadata) = read_metadata(&paths)? else {
        println!("Stoker UI is not running.");
        return Ok(());
    };
    let token = read_token(&paths)?;
    if probe_host(connect_host(metadata.host), metadata.port).is_err() {
        remove_if_exists(&paths.ui_metadata())?;
        print_notice("Stoker UI was not reachable; removed stale metadata.");
        return Ok(());
    }
    let request = format!(
        "POST /__stoker/shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    let response = request_blocking(
        connect_host(metadata.host),
        metadata.port,
        request.as_bytes(),
    )?;
    if !response.starts_with(b"HTTP/1.1 200") {
        anyhow::bail!("UI server rejected stop request")
    }
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if probe_host(connect_host(metadata.host), metadata.port).is_err() {
            print_success("Stoker UI stopped.");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("UI server did not stop within 5 seconds")
}

pub fn run(paths: StokerPaths, host: IpAddr, port: u16) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("create UI runtime")?;
    runtime.block_on(run_async(paths, host, port))
}

async fn run_async(paths: StokerPaths, host: IpAddr, port: u16) -> anyhow::Result<()> {
    paths.ensure()?;
    let listener = TcpListener::bind(SocketAddr::new(host, port))
        .await
        .with_context(|| format!("bind Stoker UI at {host}:{port}"))?;
    let actual = listener.local_addr().context("read Stoker UI address")?;
    let token = std::env::var(UI_TOKEN_ENV).ok();
    let metadata = UiMetadata {
        pid: std::process::id(),
        host,
        port: actual.port(),
        auth_required: !host.is_loopback(),
    };
    write_metadata(&paths, &metadata)?;
    let state = UiServerState {
        paths: paths.clone(),
        metadata,
        token,
        shutdown: Arc::new(Notify::new()),
        stopping: Arc::new(AtomicBool::new(false)),
    };

    let result = loop {
        tokio::select! {
            _ = state.shutdown.notified() => break Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept Stoker UI connection")?;
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, state).await;
                });
            }
        }
    };
    remove_if_exists(&paths.ui_metadata())?;
    result
}

async fn handle_connection(mut stream: TcpStream, state: UiServerState) -> anyhow::Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(error) => {
            send_response(
                &mut stream,
                HttpStatus::BAD_REQUEST,
                "application/json; charset=utf-8",
                &serde_json::to_vec(&ErrorResponse {
                    error: error.to_string(),
                })?,
            )
            .await?;
            return Ok(());
        }
    };

    if request.target == "/__stoker/shutdown" {
        if request.method != "POST" {
            send_text(&mut stream, HttpStatus::METHOD_NOT_ALLOWED, "POST required").await?;
            return Ok(());
        }
        if !authorized(&request, &state) {
            send_text(&mut stream, HttpStatus::UNAUTHORIZED, "UI token required").await?;
            return Ok(());
        }
        state.stopping.store(true, Ordering::Release);
        send_text(&mut stream, HttpStatus::OK, "stopping").await?;
        state.shutdown.notify_waiters();
        return Ok(());
    }

    let response = route_request(&request, &state).await;
    match response {
        Ok((status, content_type, body)) => {
            send_response(&mut stream, status, content_type, &body).await?
        }
        Err(error) => {
            let status = if error.downcast_ref::<UiUnauthorized>().is_some() {
                HttpStatus::UNAUTHORIZED
            } else if error.downcast_ref::<UiForbidden>().is_some() {
                HttpStatus::FORBIDDEN
            } else if error.downcast_ref::<UiBadRequest>().is_some() {
                HttpStatus::BAD_REQUEST
            } else if error.downcast_ref::<UiConflict>().is_some() {
                HttpStatus::CONFLICT
            } else if error.downcast_ref::<UiNotFound>().is_some()
                || error.downcast_ref::<UiPathNotFound>().is_some()
            {
                HttpStatus::NOT_FOUND
            } else if error.downcast_ref::<UiMethodNotAllowed>().is_some() {
                HttpStatus::METHOD_NOT_ALLOWED
            } else if error.downcast_ref::<UiPayloadTooLarge>().is_some() {
                HttpStatus::PAYLOAD_TOO_LARGE
            } else if error.downcast_ref::<UiUnsupportedMediaType>().is_some() {
                HttpStatus::UNSUPPORTED_MEDIA_TYPE
            } else if error.downcast_ref::<UiServiceUnavailable>().is_some() {
                HttpStatus::SERVICE_UNAVAILABLE
            } else {
                HttpStatus::INTERNAL_SERVER_ERROR
            };
            let body = serde_json::to_vec(&ErrorResponse {
                error: format!("{error:#}"),
            })?;
            send_response(
                &mut stream,
                status,
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
        }
    }
    Ok(())
}

async fn route_request(
    request: &HttpRequest,
    state: &UiServerState,
) -> anyhow::Result<(HttpStatus, &'static str, Vec<u8>)> {
    if let Some((content_type, body)) = static_asset(&request.method, &request.target)? {
        return Ok((HttpStatus::OK, content_type, body));
    }
    if !request.target.starts_with("/api/") {
        return Err(UiNotFound.into());
    }

    let (path, query) = split_target(&request.target);
    if !request_source_allowed(request) {
        return Err(UiForbidden.into());
    }
    if path != "/api/v1/ui/config" && !authorized(request, state) {
        return Err(UiUnauthorized.into());
    }

    let (status, body) = match (request.method.as_str(), path) {
        ("GET", "/api/v1/ui/config") => (
            HttpStatus::OK,
            serde_json::to_vec(&UiConfigResponse {
                auth_required: state.metadata.auth_required,
                version: env!("CARGO_PKG_VERSION"),
                max_job_name_length: MAX_JOB_NAME_LENGTH,
                max_job_user_length: MAX_JOB_USER_LENGTH,
                max_job_description_length: MAX_JOB_DESCRIPTION_LENGTH,
            })?,
        ),
        ("GET", "/api/v1/status") => (HttpStatus::OK, status_json(state).await?),
        ("GET", "/api/v1/jobs") => (HttpStatus::OK, jobs_json(state, query).await?),
        ("GET", "/api/v1/queue") => (HttpStatus::OK, queue_json(state).await?),
        ("GET", "/api/v1/fs/roots") => (HttpStatus::OK, fs_roots_json(state)?),
        ("GET", "/api/v1/fs/directories") => (HttpStatus::OK, fs_directories_json(query)?),
        ("GET", path) if path.starts_with("/api/v1/jobs/") && path.ends_with("/logs") => {
            (HttpStatus::OK, logs_json(state, job_id(path)?)?)
        }
        ("GET", path) if path.starts_with("/api/v1/jobs/") => {
            (HttpStatus::OK, job_detail_json(state, plain_job_id(path)?)?)
        }
        ("GET", "/api/v1/config") | ("GET", "/api/v1/config/snapshots") => {
            (HttpStatus::OK, configuration_json(state)?)
        }
        ("POST", "/api/v1/jobs") => (HttpStatus::CREATED, create_job_json(request, state)?),
        ("PATCH", path) if path.starts_with("/api/v1/jobs/") && path.ends_with("/description") => (
            HttpStatus::OK,
            update_description_json(request, state, action_job_id(path, "description")?)?,
        ),
        ("POST", path) if path.starts_with("/api/v1/jobs/") && path.ends_with("/commit") => (
            HttpStatus::OK,
            commit_job_json(state, action_job_id(path, "commit")?).await?,
        ),
        ("POST", path) if path.starts_with("/api/v1/jobs/") && path.ends_with("/cancel") => (
            HttpStatus::OK,
            cancel_job_json(state, action_job_id(path, "cancel")?).await?,
        ),
        ("POST", "/api/v1/clean") => (HttpStatus::OK, clean_json(state)?),
        ("POST", "/api/v1/queue/lock") => (HttpStatus::OK, queue_lock_json(state, true).await?),
        ("POST", "/api/v1/queue/unlock") => (HttpStatus::OK, queue_lock_json(state, false).await?),
        ("POST", "/api/v1/config/snapshot") => (HttpStatus::OK, create_snapshot_json(state)?),
        ("PUT", "/api/v1/config/timezone") => {
            (HttpStatus::OK, update_timezone_json(request, state)?)
        }
        ("DELETE", "/api/v1/config/timezone") => (HttpStatus::OK, unset_timezone_json(state)?),
        ("POST", "/api/v1/config/restore") => {
            (HttpStatus::OK, restore_snapshot_json(request, state)?)
        }
        ("POST", path) if path.starts_with("/api/v1/queue/") && path.ends_with("/move") => {
            let id = queue_job_id(path)?;
            (HttpStatus::OK, move_queue_json(request, state, id).await?)
        }
        (_, path) if path.starts_with("/api/v1/jobs/") && path.ends_with("/logs") => {
            return Err(UiMethodNotAllowed.into());
        }
        (_, path)
            if path.starts_with("/api/v1/jobs/")
                && (path.ends_with("/commit") || path.ends_with("/cancel")) =>
        {
            return Err(UiMethodNotAllowed.into());
        }
        (_, path) if path.starts_with("/api/v1/jobs/") => {
            return Err(UiMethodNotAllowed.into());
        }
        (_, path) if path.starts_with("/api/v1/queue/") && path.ends_with("/move") => {
            return Err(UiMethodNotAllowed.into());
        }
        (_, "/api/v1/ui/config")
        | (_, "/api/v1/status")
        | (_, "/api/v1/jobs")
        | (_, "/api/v1/queue")
        | (_, "/api/v1/fs/roots")
        | (_, "/api/v1/fs/directories")
        | (_, "/api/v1/config")
        | (_, "/api/v1/config/snapshots")
        | (_, "/api/v1/clean")
        | (_, "/api/v1/queue/lock")
        | (_, "/api/v1/queue/unlock")
        | (_, "/api/v1/config/snapshot")
        | (_, "/api/v1/config/timezone")
        | (_, "/api/v1/config/restore") => return Err(UiMethodNotAllowed.into()),
        _ => return Err(UiNotFound.into()),
    };
    Ok((status, "application/json; charset=utf-8", body))
}

fn create_job_json(request: &HttpRequest, state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    require_json_content_type(request)?;
    let body: CreateJobRequest = json_body(request)?;
    let job = crate::submission::create_shell_job(
        &Store::open(&state.paths.database)?,
        body.user,
        body.name,
        PathBuf::from(body.cwd),
        body.command,
        body.description,
    )
    .map_err(UiBadRequest::from_error)?;
    Ok(serde_json::to_vec(&CreateJobResponse { job })?)
}

fn update_description_json(
    request: &HttpRequest,
    state: &UiServerState,
    id: Uuid,
) -> anyhow::Result<Vec<u8>> {
    require_json_content_type(request)?;
    let body: UpdateDescriptionRequest = json_body(request)?;
    let store = Store::open(&state.paths.database)?;
    match store.update_description(id, body.description, body.expected_revision) {
        Ok(job) => Ok(serde_json::to_vec(&JobActionResponse { job })?),
        Err(StoreError::NotFound { .. }) => Err(UiNotFound.into()),
        Err(error @ StoreError::DescriptionConflict { .. }) => {
            Err(UiConflict::from_error(error.into()))
        }
        Err(StoreError::InvalidData(message)) => Err(UiBadRequest::new(message).into()),
        Err(error) => Err(error.into()),
    }
}

fn job_detail_json(state: &UiServerState, id: Uuid) -> anyhow::Result<Vec<u8>> {
    let job = get_job_or_not_found(state, id)?;
    let timezone = resolve_timezone(&state.paths, None)?;
    Ok(serde_json::to_vec(&JobDetailResponse {
        working_directory_status: working_directory_status(job.state),
        display_timezone: timezone.name,
        job,
    })?)
}

async fn commit_job_json(state: &UiServerState, id: Uuid) -> anyhow::Result<Vec<u8>> {
    get_job_or_not_found(state, id)?;
    ServiceClient::new(state.paths.clone())
        .commit(id)
        .await
        .map_err(action_error)?;
    let job = get_job_or_not_found(state, id)?;
    Ok(serde_json::to_vec(&JobActionResponse { job })?)
}

async fn cancel_job_json(state: &UiServerState, id: Uuid) -> anyhow::Result<Vec<u8>> {
    get_job_or_not_found(state, id)?;
    ServiceClient::new(state.paths.clone())
        .cancel(id)
        .await
        .map_err(action_error)?;
    let job = get_job_or_not_found(state, id)?;
    Ok(serde_json::to_vec(&JobActionResponse { job })?)
}

fn action_error(error: anyhow::Error) -> anyhow::Error {
    if is_service_unavailable(&error) {
        UiServiceUnavailable::from_error(anyhow::anyhow!(
            "Scheduler is not running. Run `stoker start` first."
        ))
    } else {
        UiConflict::from_error(error)
    }
}

fn get_job_or_not_found(state: &UiServerState, id: Uuid) -> anyhow::Result<Job> {
    Store::open(&state.paths.database)?
        .get_job(id)
        .map_err(|error| match error {
            StoreError::NotFound { .. } => UiNotFound.into(),
            other => anyhow::Error::from(other),
        })
}

fn working_directory_status(state: JobState) -> &'static str {
    match state {
        JobState::Draft | JobState::Queued => "planned",
        JobState::Starting | JobState::Running | JobState::Cancelling => "active",
        _ => "source directory retained",
    }
}

fn require_json_content_type(request: &HttpRequest) -> anyhow::Result<()> {
    let valid = request
        .headers
        .get("content-type")
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("application/json"));
    if !valid {
        return Err(UiUnsupportedMediaType.into());
    }
    Ok(())
}

fn fs_roots_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let startup = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .map(crate::config::normalize_path)
        .filter(|path| path.is_dir());
    let home = home_directory()
        .and_then(|path| path.canonicalize().ok())
        .map(crate::config::normalize_path)
        .filter(|path| path.is_dir());

    let mut locations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |kind: &'static str, label: String, path: PathBuf| {
        if let Some(value) = ui_path(path)
            && seen.insert(value.clone())
        {
            locations.push(FsLocation {
                kind,
                label,
                path: value,
            });
        }
    };

    if let Some(path) = home.clone() {
        add("home", "Home".to_owned(), path);
    }
    if let Some(path) = startup.clone() {
        add("startup", "UI startup directory".to_owned(), path);
    }

    let jobs = Store::open(&state.paths.database)?.list_jobs(None)?;
    for job in jobs.iter().rev().take(8) {
        if job.cwd.is_dir() {
            let label = job
                .cwd
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| format!("Recent · {value}"))
                .unwrap_or_else(|| "Recent".to_owned());
            add("recent", label, job.cwd.clone());
        }
    }

    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", char::from(letter)));
        if path.is_dir() {
            add("drive", format!("{}:", char::from(letter)), path);
        }
    }
    #[cfg(not(windows))]
    add("root", "Filesystem root".to_owned(), PathBuf::from("/"));

    let default_path = startup
        .or(home)
        .or_else(|| locations.first().map(|entry| PathBuf::from(&entry.path)))
        .and_then(ui_path);
    Ok(serde_json::to_vec(&FsRootsResponse {
        default_path,
        locations,
    })?)
}

fn fs_directories_json(query: &str) -> anyhow::Result<Vec<u8>> {
    const MAX_DIRECTORIES: usize = 500;
    const MAX_ENTRIES: usize = 5_000;
    let params = parse_query(query);
    let raw_path = params
        .get("path")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| UiBadRequest::new("path is required"))?;
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() {
        return Err(UiBadRequest::new("path must be an absolute directory").into());
    }
    let metadata = fs::metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            UiPathNotFound.into()
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            UiForbidden.into()
        } else {
            anyhow::Error::from(error)
        }
    })?;
    if !metadata.is_dir() {
        return Err(UiBadRequest::new("path is not a directory").into());
    }
    let path = path.canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => anyhow::Error::from(UiPathNotFound),
        _ => UiBadRequest::new(format!("resolve directory: {error}")).into(),
    })?;
    let parent = path.parent().map(Path::to_path_buf);
    let mut directories = Vec::new();
    let mut skipped_entries = 0;
    let mut truncated = false;
    let entries = fs::read_dir(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => anyhow::Error::from(UiPathNotFound),
        std::io::ErrorKind::PermissionDenied => anyhow::Error::from(UiForbidden),
        _ => anyhow::Error::from(error),
    })?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped_entries += 1;
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(value) => value,
            Err(_) => {
                skipped_entries += 1;
                continue;
            }
        };
        let candidate = entry.path();
        let metadata = match fs::metadata(&candidate) {
            Ok(value) => value,
            Err(_) => {
                skipped_entries += 1;
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            skipped_entries += 1;
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        if directories.len() >= MAX_DIRECTORIES {
            truncated = true;
            break;
        }
        let resolved = match candidate.canonicalize() {
            Ok(value) => crate::config::normalize_path(value),
            Err(_) => {
                skipped_entries += 1;
                continue;
            }
        };
        let Some(path) = ui_path(resolved) else {
            skipped_entries += 1;
            continue;
        };
        directories.push(FsDirectory {
            name,
            path,
            is_symlink: file_type.is_symlink(),
        });
    }
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.name.cmp(&right.name))
    });
    Ok(serde_json::to_vec(&FsDirectoriesResponse {
        path: ui_path(path)
            .ok_or_else(|| UiBadRequest::new("directory path is not valid UTF-8"))?,
        parent: parent.map(crate::config::normalize_path).and_then(ui_path),
        directories,
        truncated,
        skipped_entries,
    })?)
}

fn ui_path(path: PathBuf) -> Option<String> {
    let path = crate::config::normalize_path(path);
    path.to_str().map(|value| {
        #[cfg(windows)]
        {
            value.replace('\\', "/")
        }
        #[cfg(not(windows))]
        {
            value.to_owned()
        }
    })
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

async fn status_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let jobs = Store::open(&state.paths.database)?.list_jobs(None)?;
    let timezone = resolve_timezone(&state.paths, None)?;
    let queue_locked = Store::open(&state.paths.database)?.queue_locked()?;
    let scheduler = match ServiceClient::new(state.paths.clone()).status().await {
        Ok(service) => SchedulerResponse {
            running: true,
            pid: Some(service.pid),
            active_job: service.active_job,
            queued_jobs: service.queued_jobs,
        },
        Err(error) if is_service_unavailable(&error) => SchedulerResponse {
            running: false,
            pid: None,
            active_job: None,
            queued_jobs: jobs
                .iter()
                .filter(|job| job.state == JobState::Queued)
                .count(),
        },
        Err(error) => return Err(error),
    };
    let counts = CountResponse {
        total: jobs.len(),
        draft: count_state(&jobs, JobState::Draft),
        queued: count_state(&jobs, JobState::Queued),
        active: jobs
            .iter()
            .filter(|job| {
                matches!(
                    job.state,
                    JobState::Starting | JobState::Running | JobState::Cancelling
                )
            })
            .count(),
        succeeded: count_state(&jobs, JobState::Succeeded),
        failed: jobs
            .iter()
            .filter(|job| matches!(job.state, JobState::Failed | JobState::Lost))
            .count(),
    };
    Ok(serde_json::to_vec(&StatusResponse {
        scheduler,
        counts,
        queue_locked,
        timezone: timezone_response(&timezone),
        generated_at: Utc::now().to_rfc3339(),
    })?)
}

async fn jobs_json(state: &UiServerState, query: &str) -> anyhow::Result<Vec<u8>> {
    let timezone = resolve_timezone(&state.paths, None)?;
    let params = parse_query(query);
    let owner = params
        .get("user")
        .map(String::as_str)
        .filter(|value| !value.is_empty());
    let job_state = params
        .get("state")
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_uppercase().parse::<JobState>())
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    let jobs = Store::open(&state.paths.database)?.list_jobs_with_state(owner, job_state)?;
    Ok(serde_json::to_vec(&JobsResponse {
        jobs,
        timezone: timezone_response(&timezone),
    })?)
}

async fn queue_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let store = Store::open(&state.paths.database)?;
    let jobs = store.list_jobs_with_state(None, Some(JobState::Queued))?;
    Ok(serde_json::to_vec(&QueueResponse {
        jobs,
        locked: store.queue_locked()?,
    })?)
}

fn clean_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let jobs = Store::open(&state.paths.database)?.clean_terminal_jobs()?;
    for job in &jobs {
        let run_dir = state.paths.runs.join(job.id.to_string());
        if run_dir.exists() {
            fs::remove_dir_all(&run_dir)
                .with_context(|| format!("remove logs for job {}", job.id))?;
        }
    }
    Ok(serde_json::to_vec(&CleanResponse {
        removed: jobs.len(),
    })?)
}

fn logs_json(state: &UiServerState, id: Uuid) -> anyhow::Result<Vec<u8>> {
    let store = Store::open(&state.paths.database)?;
    let job = match store.get_job(id) {
        Ok(job) => job,
        Err(StoreError::NotFound { .. }) => return Err(UiNotFound.into()),
        Err(error) => return Err(error.into()),
    };
    let run_dir = state.paths.runs.join(id.to_string());
    let (stdout, stdout_available, stdout_truncated) = read_log(&run_dir.join("stdout.log"))?;
    let (stderr, stderr_available, stderr_truncated) = read_log(&run_dir.join("stderr.log"))?;
    let available = stdout_available || stderr_available;
    let message = if job.state == JobState::Draft {
        Some(format!(
            "Job {id} is still DRAFT; commit it before the scheduler can create logs."
        ))
    } else if job.state == JobState::Queued {
        Some(format!(
            "Job {id} is QUEUED; logs will be available after the scheduler starts it."
        ))
    } else if !available {
        Some(format!("No logs are available for job {id} yet."))
    } else {
        None
    };
    Ok(serde_json::to_vec(&LogsResponse {
        job,
        stdout,
        stderr,
        stdout_available,
        stderr_available,
        stdout_truncated,
        stderr_truncated,
        message,
    })?)
}

fn read_log(path: &Path) -> anyhow::Result<(String, bool, bool)> {
    if !path.exists() {
        return Ok((String::new(), false, false));
    }
    let mut bytes = fs::read(path).with_context(|| format!("read log {}", path.display()))?;
    let truncated = bytes.len() > MAX_LOG_BYTES;
    if truncated {
        let start = bytes.len() - MAX_LOG_BYTES;
        bytes = bytes.split_off(start);
    }
    Ok((
        String::from_utf8_lossy(&bytes).into_owned(),
        true,
        truncated,
    ))
}

fn configuration_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let config = state.paths.read_config()?;
    let timezone = resolve_timezone(&state.paths, None)?;
    let snapshots = state
        .paths
        .list_config_snapshots()?
        .into_iter()
        .map(|entry| match entry {
            crate::config::ConfigSnapshotEntry::Valid(snapshot) => SnapshotResponse {
                path: snapshot.path.to_string_lossy().into_owned(),
                valid: true,
                created_at: Some(snapshot.snapshot.created_at.to_rfc3339()),
                reason: Some(snapshot.snapshot.reason.to_string()),
                timezone: snapshot.snapshot.config.timezone,
                error: None,
            },
            crate::config::ConfigSnapshotEntry::Invalid { path, error } => SnapshotResponse {
                path: path.to_string_lossy().into_owned(),
                valid: false,
                created_at: None,
                reason: None,
                timezone: None,
                error: Some(error),
            },
        })
        .collect();
    Ok(serde_json::to_vec(&ConfigurationResponse {
        config,
        effective_timezone: timezone_response(&timezone),
        timezones: chrono_tz::TZ_VARIANTS
            .iter()
            .map(ToString::to_string)
            .collect(),
        config_path: state.paths.config_path().to_string_lossy().into_owned(),
        snapshot_dir: state.paths.snapshot_dir().to_string_lossy().into_owned(),
        snapshots,
    })?)
}

async fn queue_lock_json(state: &UiServerState, lock: bool) -> anyhow::Result<Vec<u8>> {
    let client = ServiceClient::new(state.paths.clone());
    let result = if lock {
        client.lock_queue().await
    } else {
        client.unlock_queue().await
    };
    match result {
        Ok(()) => {}
        Err(error) if is_service_unavailable(&error) => {
            let store = Store::open(&state.paths.database)?;
            if lock {
                store.lock_queue()?;
            } else {
                store.unlock_queue()?;
            }
        }
        Err(error) => return Err(UiConflict::from_error(error)),
    }
    queue_json(state).await
}

async fn move_queue_json(
    request: &HttpRequest,
    state: &UiServerState,
    id: Uuid,
) -> anyhow::Result<Vec<u8>> {
    let body: QueueMoveRequest = json_body(request)?;
    let client = ServiceClient::new(state.paths.clone());
    let result = match client.status().await {
        Ok(_) => client.move_queued(id, body.target_order).await,
        Err(error) if is_service_unavailable(&error) => Store::open(&state.paths.database)?
            .move_queued_job(id, body.target_order)
            .map_err(anyhow::Error::from),
        Err(error) => return Err(UiServiceUnavailable::from_error(error)),
    };
    match result {
        Ok(_) => queue_json(state).await,
        Err(error) if is_service_unavailable(&error) => {
            Store::open(&state.paths.database)?
                .move_queued_job(id, body.target_order)
                .map_err(|error| UiConflict::from_error(error.into()))?;
            queue_json(state).await
        }
        Err(error) => Err(UiConflict::from_error(error)),
    }
}

fn update_timezone_json(request: &HttpRequest, state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let body: TimezoneRequest = json_body(request)?;
    let value = body.value.trim();
    if value.is_empty() {
        return Err(UiBadRequest::new("timezone cannot be empty").into());
    }
    resolve_timezone(&state.paths, Some(value)).map_err(UiBadRequest::from_error)?;
    let mut config = state.paths.read_config()?;
    config.timezone = Some(value.to_owned());
    state.paths.write_config(&config)?;
    configuration_json(state)
}

fn unset_timezone_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let mut config = state.paths.read_config()?;
    config.timezone = None;
    state.paths.write_config(&config)?;
    configuration_json(state)
}

fn create_snapshot_json(state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let config = state.paths.read_config()?;
    state
        .paths
        .create_config_snapshot(&config, crate::config::ConfigSnapshotReason::Manual)?;
    configuration_json(state)
}

fn restore_snapshot_json(request: &HttpRequest, state: &UiServerState) -> anyhow::Result<Vec<u8>> {
    let body: RestoreRequest = json_body(request)?;
    let entry = state
        .paths
        .list_config_snapshots()?
        .into_iter()
        .find(|entry| match entry {
            crate::config::ConfigSnapshotEntry::Valid(snapshot) => {
                snapshot.path.to_string_lossy() == body.path
            }
            crate::config::ConfigSnapshotEntry::Invalid { path, .. } => {
                path.to_string_lossy() == body.path
            }
        });
    let Some(entry) = entry else {
        return Err(UiNotFound.into());
    };
    let crate::config::ConfigSnapshotEntry::Valid(snapshot) = entry else {
        return Err(UiConflict::new("the selected configuration snapshot is invalid").into());
    };
    state.paths.restore_config_snapshot(&snapshot.snapshot)?;
    configuration_json(state)
}

fn job_id(path: &str) -> anyhow::Result<Uuid> {
    let value = path
        .strip_prefix("/api/v1/jobs/")
        .and_then(|value| value.strip_suffix("/logs"))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or(UiNotFound)?;
    Uuid::parse_str(value)
        .map_err(|error| UiBadRequest::new(format!("invalid job id: {error}")).into())
}

fn plain_job_id(path: &str) -> anyhow::Result<Uuid> {
    let value = path
        .strip_prefix("/api/v1/jobs/")
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or(UiNotFound)?;
    Uuid::parse_str(value)
        .map_err(|error| UiBadRequest::new(format!("invalid job id: {error}")).into())
}

fn action_job_id(path: &str, action: &str) -> anyhow::Result<Uuid> {
    let suffix = format!("/{action}");
    let value = path
        .strip_prefix("/api/v1/jobs/")
        .and_then(|value| value.strip_suffix(&suffix))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or(UiNotFound)?;
    Uuid::parse_str(value)
        .map_err(|error| UiBadRequest::new(format!("invalid job id: {error}")).into())
}

fn queue_job_id(path: &str) -> anyhow::Result<Uuid> {
    let value = path
        .strip_prefix("/api/v1/queue/")
        .and_then(|value| value.strip_suffix("/move"))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or(UiNotFound)?;
    Uuid::parse_str(value)
        .map_err(|error| UiBadRequest::new(format!("invalid queue job id: {error}")).into())
}

fn json_body<T: DeserializeOwned>(request: &HttpRequest) -> anyhow::Result<T> {
    serde_json::from_slice(&request.body)
        .map_err(|error| UiBadRequest::new(format!("invalid JSON request body: {error}")).into())
}

fn static_asset(method: &str, target: &str) -> anyhow::Result<Option<(&'static str, Vec<u8>)>> {
    if method != "GET" {
        return Ok(None);
    }
    let path = split_target(target).0;
    let asset = match path {
        "/" | "/index.html" => Some(("text/html; charset=utf-8", INDEX_HTML.as_bytes())),
        "/styles.css" => Some(("text/css; charset=utf-8", STYLES_CSS.as_bytes())),
        "/app.js" => Some(("text/javascript; charset=utf-8", APP_JS.as_bytes())),
        "/assets/logo.svg" => Some(("image/svg+xml", LOGO_SVG.as_bytes())),
        "/assets/logo-mark.png" => Some(("image/png", LOGO_MARK_PNG)),
        _ => None,
    };
    Ok(asset.map(|(content_type, body)| (content_type, body.to_vec())))
}

async fn read_request(stream: &mut TcpStream) -> anyhow::Result<HttpRequest> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end;
    loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.context("read HTTP request")?;
        if read == 0 {
            anyhow::bail!("connection closed before HTTP headers were complete");
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(UiPayloadTooLarge.into());
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(UiPayloadTooLarge.into());
        }
    }
    let header_text =
        std::str::from_utf8(&bytes[..header_end]).context("HTTP headers are not UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().context("missing HTTP request line")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .context("missing HTTP method")?
        .to_owned();
    let target = request_parts
        .next()
        .context("missing HTTP target")?
        .to_owned();
    let version = request_parts.next().context("missing HTTP version")?;
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        anyhow::bail!("unsupported HTTP version {version}");
    }
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').context("malformed HTTP header")?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("invalid HTTP content length")?
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BYTES || header_end + content_length > MAX_REQUEST_BYTES {
        return Err(UiPayloadTooLarge.into());
    }
    while bytes.len() < header_end + content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.context("read HTTP body")?;
        if read == 0 {
            anyhow::bail!("connection closed before HTTP body was complete");
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        target,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

async fn send_response(
    stream: &mut TcpStream,
    status: HttpStatus,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status.code,
        status.reason,
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn send_text(stream: &mut TcpStream, status: HttpStatus, text: &str) -> anyhow::Result<()> {
    send_response(stream, status, "text/plain; charset=utf-8", text.as_bytes()).await
}

fn authorized(request: &HttpRequest, state: &UiServerState) -> bool {
    if !state.metadata.auth_required {
        return true;
    }
    let Some(expected) = state.token.as_deref() else {
        return false;
    };
    let Some(header) = request.headers.get("authorization") else {
        return false;
    };
    let Some(received) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_equal(expected.as_bytes(), received.as_bytes())
}

/// Reject browser cross-origin writes/reads while retaining compatibility with
/// non-browser local clients that omit Origin. The browser's Origin authority
/// must match the HTTP Host authority exactly; bearer authentication still
/// protects LAN mode.
fn request_source_allowed(request: &HttpRequest) -> bool {
    let Some(origin) = request.headers.get("origin") else {
        return true;
    };
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") || authority.is_empty() || authority.contains('/') {
        return false;
    }
    request
        .headers
        .get("host")
        .is_some_and(|host| host.eq_ignore_ascii_case(authority))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= (left.get(index).copied().unwrap_or(0)
            ^ right.get(index).copied().unwrap_or(0)) as usize;
    }
    difference == 0
}

fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    let mut output = Vec::with_capacity(value.len());
    let mut chars = value.bytes();
    while let Some(byte) = chars.next() {
        match byte {
            b'+' => output.push(b' '),
            b'%' => {
                let high = char::from(chars.next()?).to_digit(16)?;
                let low = char::from(chars.next()?).to_digit(16)?;
                output.push((high * 16 + low) as u8);
            }
            _ => output.push(byte),
        }
    }
    String::from_utf8(output).ok()
}

fn timezone_response(timezone: &ResolvedTimezone) -> TimezoneResponse {
    TimezoneResponse {
        name: timezone.name.clone(),
        source: match timezone.source {
            TimezoneSource::Cli => "cli",
            TimezoneSource::Config => "config",
            TimezoneSource::System => "system",
        },
    }
}

fn count_state(jobs: &[Job], state: JobState) -> usize {
    jobs.iter().filter(|job| job.state == state).count()
}

fn read_metadata(paths: &StokerPaths) -> anyhow::Result<Option<UiMetadata>> {
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

fn write_token(paths: &StokerPaths, token: &str) -> anyhow::Result<()> {
    let path = paths.ui_token();
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open UI token {}", path.display()))?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn read_token(paths: &StokerPaths) -> anyhow::Result<String> {
    fs::read_to_string(paths.ui_token())
        .with_context(|| format!("read UI token {}", paths.ui_token().display()))
        .map(|token| token.trim().to_owned())
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
    std::io::Read::read_to_end(&mut stream, &mut response)?;
    Ok(response)
}

fn terminate_child(child: &mut std::process::Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn print_start_message(
    metadata: &UiMetadata,
    url: &str,
    token: &str,
    open: bool,
) -> anyhow::Result<()> {
    print_success("Stoker UI started.");
    println!("URL: {url}");
    println!("PID: {}", metadata.pid);
    if metadata.auth_required {
        println!("LAN token: {token}");
        println!("Share the token only with trusted users on this network.");
    }
    if open {
        let browser_url = if metadata.auth_required {
            format!("{url}#token={token}")
        } else {
            url.to_owned()
        };
        open_browser(&browser_url)?;
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

#[derive(Debug, thiserror::Error)]
#[error("UI authentication failed")]
struct UiUnauthorized;

#[derive(Debug, thiserror::Error)]
#[error("UI request is forbidden")]
struct UiForbidden;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct UiBadRequest {
    message: String,
}

impl UiBadRequest {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_error(error: anyhow::Error) -> Self {
        Self::new(format!("{error:#}"))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct UiConflict {
    message: String,
}

impl UiConflict {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_error(error: anyhow::Error) -> anyhow::Error {
        Self::new(format!("{error:#}")).into()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct UiServiceUnavailable {
    message: String,
}

impl UiServiceUnavailable {
    fn from_error(error: anyhow::Error) -> anyhow::Error {
        Self {
            message: format!("{error:#}"),
        }
        .into()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("UI route not found")]
struct UiNotFound;

#[derive(Debug, thiserror::Error)]
#[error("working directory not found")]
struct UiPathNotFound;

#[derive(Debug, thiserror::Error)]
#[error("HTTP method is not allowed")]
struct UiMethodNotAllowed;

#[derive(Debug, thiserror::Error)]
#[error("HTTP request is too large")]
struct UiPayloadTooLarge;

#[derive(Debug, thiserror::Error)]
#[error("request Content-Type must be application/json")]
struct UiUnsupportedMediaType;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::{Arc, atomic::AtomicBool};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::Notify;

    use crate::Store;
    use crate::config::{StokerConfig, resolve_timezone};
    use crate::domain::{JobState, MAX_JOB_DESCRIPTION_LENGTH, NewJob};

    use super::{
        APP_JS, HttpRequest, INDEX_HTML, LOGO_MARK_PNG, MAX_LOG_BYTES, STYLES_CSS, StokerPaths,
        UiBadRequest, UiConflict, UiForbidden, UiMetadata, UiMethodNotAllowed, UiNotFound,
        UiPathNotFound, UiServerState, UiServiceUnavailable, UiUnsupportedMediaType, authorized,
        connect_host, constant_time_equal, count_state, default_port, handle_connection, job_id,
        json_body, parse_query, percent_decode, plain_job_id, queue_job_id, read_log,
        read_metadata, read_token, remove_if_exists, route_request, run_async, split_target, start,
        static_asset, status, stop, timezone_response, ui_url, write_metadata, write_token,
    };

    #[test]
    fn embedded_ui_contains_jobs_queue_and_configuration_controls() {
        assert!(INDEX_HTML.contains("Configuration"));
        assert!(APP_JS.contains("Job name"));
        assert!(APP_JS.contains("maxJobNameLength"));
        assert!(APP_JS.contains("maxlength=\"${maxJobNameLength}\""));
        assert!(APP_JS.contains("max_job_name_length"));
        assert!(APP_JS.contains("maxJobUserLength"));
        assert!(APP_JS.contains("max_job_user_length"));
        assert!(APP_JS.contains("maxJobDescriptionLength"));
        assert!(APP_JS.contains("max_job_description_length"));
        assert!(APP_JS.contains("128"));
        assert!(APP_JS.contains("/api/v1/jobs"));
        assert!(APP_JS.contains("/api/v1/fs/directories"));
        assert!(APP_JS.contains("Choose working directory"));
        assert!(APP_JS.contains("Path"));
        assert!(APP_JS.contains("captureFocus"));
        assert!(APP_JS.contains("refreshLiveTimes"));
        assert!(APP_JS.contains("timeZone"));
        assert!(APP_JS.contains("configurationDraft"));
        assert!(APP_JS.contains("openConfirmation"));
        assert!(!APP_JS.contains("window.confirm"));
        assert!(APP_JS.contains("queue-lock"));
        assert!(APP_JS.contains("/api/v1/clean"));
        assert!(APP_JS.contains("Clean job history"));
        assert!(INDEX_HTML.contains("/assets/logo-mark.png"));
        assert!(LOGO_MARK_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(APP_JS.contains("/api/v1/config/timezone"));
        assert!(APP_JS.contains("timezoneSet.disabled = !exact || !changed"));
        assert!(APP_JS.contains("/api/v1/config/restore"));
        assert!(!APP_JS.contains("How to use Logs"));
        assert!(APP_JS.contains("/logs"));
        assert!(APP_JS.contains("openJobDetail"));
        assert!(APP_JS.contains("runJobAction"));
        assert!(APP_JS.contains("job-description-editor"));
        assert!(APP_JS.contains("/description"));
        assert!(APP_JS.contains("description_revision"));
        assert!(APP_JS.contains("timeline-time"));
        assert!(APP_JS.contains("job-detail-copy-id"));
        assert!(APP_JS.contains("copyJobId"));
        assert!(APP_JS.contains("timeline-heading"));
        assert!(APP_JS.contains("Commit job"));
        assert!(APP_JS.contains("Cancel job"));
        assert!(APP_JS.contains("job-suggestions"));
        assert!(APP_JS.contains("Known owners"));
        assert!(APP_JS.contains("destructive"));
        assert!(APP_JS.contains("event.target === jobDialog"));
        assert!(APP_JS.contains("event.target === jobDetailDialog"));
        assert!(APP_JS.contains("prefetchDirectory"));
        assert!(APP_JS.contains("directoryCache"));
        assert!(APP_JS.contains("JOBS_PAGE_SIZE"));
        assert!(APP_JS.contains("SNAPSHOTS_PAGE_SIZE"));
        assert!(APP_JS.contains("JOBS_PAGE_SIZE = 6"));
        assert!(APP_JS.contains("SNAPSHOTS_PAGE_SIZE = 5"));
        assert!(APP_JS.contains("data-page-kind"));
        assert!(APP_JS.contains("paginationMarkup(\"snapshots\""));
        assert!(APP_JS.contains("log-job-search"));
        assert!(STYLES_CSS.contains(".jobs-table"));
        assert!(STYLES_CSS.contains(".queue-order-table"));
        assert!(STYLES_CSS.contains(".queue-table { max-height: 560px"));
        assert!(
            STYLES_CSS.contains(".main-content { height: 100vh; min-height: 0; overflow: auto; }")
        );
        assert!(
            STYLES_CSS.contains(".sidebar { height: 100vh; min-height: 0; overflow-y: auto; }")
        );
        assert!(STYLES_CSS.contains(".list-pagination"));
        assert!(STYLES_CSS.contains(".log-job-search"));
        assert!(STYLES_CSS.contains(".snapshot-list { height: 300px"));
        assert!(STYLES_CSS.contains(".log-output"));
        assert!(STYLES_CSS.contains(".confirm-dialog"));
        assert!(STYLES_CSS.contains(".button.danger"));
        assert!(INDEX_HTML.contains("confirm-dialog"));
        assert!(INDEX_HTML.contains("job-dialog"));
        assert!(INDEX_HTML.contains("job-detail-dialog"));
        assert!(INDEX_HTML.contains("id=\"job-dialog\" tabindex=\"-1\""));
        assert!(STYLES_CSS.contains(".job-dialog"));
        assert!(STYLES_CSS.contains(".job-detail-dialog"));
        assert!(STYLES_CSS.contains(".job-suggestions"));
        assert!(STYLES_CSS.contains(".job-description-preview"));
        assert!(STYLES_CSS.contains(".job-timeline"));
        assert!(STYLES_CSS.contains(".timeline-time"));
        assert!(STYLES_CSS.contains(".copy-id-button"));
    }

    #[test]
    fn ui_helpers_cover_assets_paths_metadata_and_timezone_sources() {
        assert_eq!(default_port(), 8765);

        for (target, content_type) in [
            ("/", "text/html; charset=utf-8"),
            ("/index.html?cache=1", "text/html; charset=utf-8"),
            ("/styles.css", "text/css; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/assets/logo.svg", "image/svg+xml"),
            ("/assets/logo-mark.png", "image/png"),
        ] {
            let asset = static_asset("GET", target).unwrap().unwrap();
            assert_eq!(asset.0, content_type);
            assert!(!asset.1.is_empty());
        }
        assert!(static_asset("POST", "/").unwrap().is_none());
        assert!(static_asset("GET", "/missing").unwrap().is_none());

        let id = uuid::Uuid::new_v4();
        assert_eq!(job_id(&format!("/api/v1/jobs/{id}/logs")).unwrap(), id);
        assert_eq!(plain_job_id(&format!("/api/v1/jobs/{id}")).unwrap(), id);
        assert!(
            job_id("/api/v1/jobs//logs")
                .unwrap_err()
                .downcast_ref::<UiNotFound>()
                .is_some()
        );
        assert!(
            job_id("/api/v1/jobs/not-a-uuid/logs")
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );
        assert_eq!(
            queue_job_id(&format!("/api/v1/queue/{id}/move")).unwrap(),
            id
        );
        assert!(
            queue_job_id("/api/v1/queue/not-a-uuid/move")
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );

        let valid_request = HttpRequest {
            method: "PUT".into(),
            target: "/api/v1/config/timezone".into(),
            headers: HashMap::new(),
            body: br#"{"value":"UTC"}"#.to_vec(),
        };
        let body: serde_json::Value = json_body(&valid_request).unwrap();
        assert_eq!(body["value"], "UTC");
        let invalid_request = HttpRequest {
            body: b"not-json".to_vec(),
            ..valid_request
        };
        assert!(
            json_body::<serde_json::Value>(&invalid_request)
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );

        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let cli_timezone = resolve_timezone(&paths, Some("UTC")).unwrap();
        assert_eq!(timezone_response(&cli_timezone).source, "cli");
        paths
            .write_config(&StokerConfig {
                timezone: Some("Asia/Tokyo".into()),
            })
            .unwrap();
        let config_timezone = resolve_timezone(&paths, None).unwrap();
        assert_eq!(timezone_response(&config_timezone).source, "config");
        paths.write_config(&StokerConfig::default()).unwrap();
        let system_timezone = resolve_timezone(&paths, None).unwrap();
        assert_eq!(timezone_response(&system_timezone).source, "system");

        assert_eq!(
            connect_host("127.0.0.1".parse().unwrap()),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            connect_host("0.0.0.0".parse().unwrap()),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            connect_host("::".parse().unwrap()),
            "::1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            ui_url(&UiMetadata {
                pid: 1,
                host: "::".parse().unwrap(),
                port: 8765,
                auth_required: false,
            }),
            "http://[::1]:8765"
        );

        assert!(read_metadata(&paths).unwrap().is_none());
        let metadata = UiMetadata {
            pid: 42,
            host: "127.0.0.1".parse().unwrap(),
            port: 9000,
            auth_required: false,
        };
        write_metadata(&paths, &metadata).unwrap();
        assert_eq!(read_metadata(&paths).unwrap().unwrap().pid, 42);
        write_token(&paths, " secret-token ").unwrap();
        assert_eq!(read_token(&paths).unwrap(), "secret-token");
        remove_if_exists(&paths.ui_metadata()).unwrap();
        remove_if_exists(&paths.ui_metadata()).unwrap();

        assert_eq!(count_state(&[], JobState::Draft), 0);
        assert_eq!(
            UiBadRequest::from_error(anyhow::anyhow!("bad")).to_string(),
            "bad"
        );
        assert_eq!(
            UiConflict::from_error(anyhow::anyhow!("conflict")).to_string(),
            "conflict"
        );
        assert_eq!(
            UiServiceUnavailable::from_error(anyhow::anyhow!("offline")).to_string(),
            "offline"
        );
    }

    #[test]
    fn ui_lifecycle_handles_already_running_and_stale_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        write_metadata(
            &paths,
            &UiMetadata {
                pid: 99,
                host: "127.0.0.1".parse().unwrap(),
                port,
                auth_required: false,
            },
        )
        .unwrap();

        start(paths.clone(), "127.0.0.1".parse().unwrap(), port, false).unwrap();
        status(paths.clone()).unwrap();
        drop(listener);

        write_token(&paths, "stale-token").unwrap();
        status(paths.clone()).unwrap();
        stop(paths.clone()).unwrap();
        assert!(!paths.ui_metadata().exists());
        status(paths.clone()).unwrap();
        stop(paths).unwrap();
    }

    #[tokio::test]
    async fn ui_stop_requests_shutdown_and_cleans_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        write_token(&paths, "loopback-token").unwrap();
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

    #[tokio::test]
    async fn http_error_responses_cover_parser_auth_and_status_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();

        let response = one_shot_request(state_for(&paths, false, None), "GET\r\n\r\n").await;
        assert!(response.starts_with(b"HTTP/1.1 400 Bad Request"));

        let response = one_shot_request(
            state_for(&paths, false, None),
            "GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 404 Not Found"));

        let response = one_shot_request(
            state_for(&paths, false, None),
            "POST /api/v1/status HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 405 Method Not Allowed"));

        let response = one_shot_request(
            state_for(&paths, false, None),
            "GET / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with(b"HTTP/1.1 400 Bad Request"),
            "{}",
            String::from_utf8_lossy(&response)
        );

        let response = one_shot_request(
            state_for(&paths, true, Some("secret")),
            "GET /api/v1/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 401 Unauthorized"));

        let mut bad_paths = paths.clone();
        bad_paths.database = directory.path().to_path_buf();
        let response = one_shot_request(
            state_for(&bad_paths, false, None),
            "GET /api/v1/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 500 Internal Server Error"));

        let response = one_shot_request(
            state_for(&paths, false, None),
            "GET /__stoker/shutdown HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 405 Method Not Allowed"));
    }

    #[tokio::test]
    async fn route_validation_covers_logs_filters_snapshots_and_methods() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths, false, None);

        let request = make_request("GET", "/outside", "");
        assert!(
            route_request(&request, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiNotFound>()
                .is_some()
        );
        let request = make_request("GET", "/api/v1/jobs?state=not-a-state", "");
        assert!(
            route_request(&request, &state)
                .await
                .unwrap_err()
                .to_string()
                .contains("unknown job state")
        );
        let request = make_request("GET", "/api/v1/jobs?user=&state=", "");
        assert!(route_request(&request, &state).await.is_ok());

        let missing_job = uuid::Uuid::new_v4();
        let request = make_request("GET", &format!("/api/v1/jobs/{missing_job}/logs"), "");
        assert!(
            route_request(&request, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiNotFound>()
                .is_some()
        );
        let request = make_request("GET", "/api/v1/jobs/not-a-uuid/logs", "");
        assert!(
            route_request(&request, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );
        let request = make_request("POST", "/api/v1/queue/not-a-uuid/move", "{}");
        assert!(
            route_request(&request, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );

        let draft = Store::open(&paths.database)
            .unwrap()
            .create_job(NewJob {
                name: "queued-log".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into()],
            })
            .unwrap();
        Store::open(&paths.database)
            .unwrap()
            .commit_job(draft)
            .unwrap();
        let request = make_request("GET", &format!("/api/v1/jobs/{draft}/logs"), "");
        let queued_logs = route_request(&request, &state).await.unwrap().2;
        assert!(String::from_utf8_lossy(&queued_logs).contains("is QUEUED"));

        let cancelled = Store::open(&paths.database)
            .unwrap()
            .create_job(NewJob {
                name: "cancelled-log".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into()],
            })
            .unwrap();
        Store::open(&paths.database)
            .unwrap()
            .cancel_not_started(cancelled)
            .unwrap();
        let request = make_request("GET", &format!("/api/v1/jobs/{cancelled}/logs"), "");
        let missing_logs = route_request(&request, &state).await.unwrap().2;
        assert!(String::from_utf8_lossy(&missing_logs).contains("No logs are available"));

        let empty_timezone = make_request("PUT", "/api/v1/config/timezone", r#"{"value":"  "}"#);
        assert!(
            route_request(&empty_timezone, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );
        let bad_restore = make_request(
            "POST",
            "/api/v1/config/restore",
            r#"{"path":"missing.json"}"#,
        );
        assert!(
            route_request(&bad_restore, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiNotFound>()
                .is_some()
        );
        let invalid_restore = make_request(
            "POST",
            "/api/v1/config/restore",
            r#"{"path":"missing.json""#,
        );
        assert!(
            route_request(&invalid_restore, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiBadRequest>()
                .is_some()
        );

        std::fs::write(paths.snapshot_dir().join("broken.json"), "not-json").unwrap();
        let configuration = make_request("GET", "/api/v1/config/snapshots", "");
        let configuration = route_request(&configuration, &state).await.unwrap().2;
        assert!(String::from_utf8_lossy(&configuration).contains("\"valid\":false"));

        for (method, target) in [
            ("POST", "/api/v1/jobs/not-a-real-id/logs"),
            ("DELETE", "/api/v1/queue/not-a-real-id/move"),
            ("POST", "/api/v1/status"),
        ] {
            let request = make_request(method, target, "");
            assert!(
                route_request(&request, &state)
                    .await
                    .unwrap_err()
                    .downcast_ref::<UiMethodNotAllowed>()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn job_detail_and_actions_expose_show_data_and_service_errors() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths, false, None);
        let job = Store::open(&paths.database)
            .unwrap()
            .create_job(NewJob {
                name: "detail-job".into(),
                user: "alice".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "hello world".into()],
            })
            .unwrap();

        let detail = route_request(
            &make_request("GET", &format!("/api/v1/jobs/{job}"), ""),
            &state,
        )
        .await
        .unwrap();
        assert_eq!(detail.0.code, 200);
        let detail: serde_json::Value = serde_json::from_slice(&detail.2).unwrap();
        assert_eq!(detail["job"]["id"], job.to_string());
        assert_eq!(detail["job"]["name"], "detail-job");
        assert_eq!(detail["job"]["state"], "DRAFT");
        assert_eq!(detail["working_directory_status"], "planned");
        assert!(detail["display_timezone"].as_str().is_some());

        let wrong_method = route_request(
            &make_request("POST", &format!("/api/v1/jobs/{job}"), ""),
            &state,
        )
        .await
        .unwrap_err();
        assert!(wrong_method.downcast_ref::<UiMethodNotAllowed>().is_some());

        let commit = route_request(
            &make_request("POST", &format!("/api/v1/jobs/{job}/commit"), ""),
            &state,
        )
        .await
        .unwrap_err();
        assert!(commit.downcast_ref::<UiServiceUnavailable>().is_some());

        let cancel = route_request(
            &make_request("POST", &format!("/api/v1/jobs/{job}/cancel"), ""),
            &state,
        )
        .await
        .unwrap_err();
        assert!(cancel.downcast_ref::<UiServiceUnavailable>().is_some());

        let missing_action = route_request(
            &make_request(
                "POST",
                &format!("/api/v1/jobs/{}/commit", uuid::Uuid::new_v4()),
                "",
            ),
            &state,
        )
        .await
        .unwrap_err();
        assert!(missing_action.downcast_ref::<UiNotFound>().is_some());

        let missing = route_request(
            &make_request("GET", &format!("/api/v1/jobs/{}", uuid::Uuid::new_v4()), ""),
            &state,
        )
        .await
        .unwrap_err();
        assert!(missing.downcast_ref::<UiNotFound>().is_some());
    }

    #[tokio::test]
    async fn filesystem_and_create_job_routes_use_server_paths_and_persist_draft() {
        let directory = tempfile::tempdir().unwrap();
        let working = directory.path().join("工作 目錄");
        std::fs::create_dir_all(working.join("子資料夾")).unwrap();
        std::fs::write(working.join("ignored.txt"), b"file").unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths, false, None);

        let roots = route_request(&make_request("GET", "/api/v1/fs/roots", ""), &state)
            .await
            .unwrap();
        assert_eq!(roots.0.code, 200);
        assert!(String::from_utf8_lossy(&roots.2).contains("locations"));

        let encoded = percent_encode_for_test(&working);
        let listing = route_request(
            &make_request("GET", &format!("/api/v1/fs/directories?path={encoded}"), ""),
            &state,
        )
        .await
        .unwrap();
        let listing: serde_json::Value = serde_json::from_slice(&listing.2).unwrap();
        assert_eq!(listing["directories"][0]["name"], "子資料夾");
        assert_eq!(listing["directories"].as_array().unwrap().len(), 1);

        let missing = route_request(
            &make_request(
                "GET",
                &format!(
                    "/api/v1/fs/directories?path={}",
                    percent_encode_for_test(&working.join("missing"))
                ),
                "",
            ),
            &state,
        )
        .await
        .unwrap_err();
        assert!(missing.downcast_ref::<UiPathNotFound>().is_some());

        let body = format!(
            r#"{{"user":"alice","name":"build","description":"Build the project","cwd":{},"command":"echo \"hello world\""}}"#,
            serde_json::to_string(&working.to_string_lossy()).unwrap()
        );
        let mut request = make_request("POST", "/api/v1/jobs", &body);
        request.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        let created = route_request(&request, &state).await.unwrap();
        assert_eq!(created.0.code, 201);
        let created: serde_json::Value = serde_json::from_slice(&created.2).unwrap();
        assert_eq!(created["job"]["state"], "DRAFT");
        assert_eq!(created["job"]["user"], "alice");
        assert_eq!(created["job"]["description"], "Build the project");
        assert_eq!(created["job"]["description_revision"], 0);
        assert_eq!(created["job"]["command_line"], "echo \"hello world\"");
        let created_id = created["job"]["id"].as_str().unwrap();
        let mut update = make_request(
            "PATCH",
            &format!("/api/v1/jobs/{created_id}/description"),
            r#"{"description":"Updated from the browser","expected_revision":0}"#,
        );
        update
            .headers
            .insert("content-type".into(), "application/json".into());
        let updated = route_request(&update, &state).await.unwrap();
        let updated: serde_json::Value = serde_json::from_slice(&updated.2).unwrap();
        assert_eq!(updated["job"]["description"], "Updated from the browser");
        assert_eq!(updated["job"]["description_revision"], 1);

        let mut stale = make_request(
            "PATCH",
            &format!("/api/v1/jobs/{created_id}/description"),
            r#"{"description":"Stale update","expected_revision":0}"#,
        );
        stale
            .headers
            .insert("content-type".into(), "application/json".into());
        let conflict = route_request(&stale, &state).await.unwrap_err();
        assert!(conflict.downcast_ref::<UiConflict>().is_some());

        let mut too_long = make_request(
            "PATCH",
            &format!("/api/v1/jobs/{created_id}/description"),
            &serde_json::json!({
                "description": "x".repeat(MAX_JOB_DESCRIPTION_LENGTH + 1),
                "expected_revision": 1
            })
            .to_string(),
        );
        too_long
            .headers
            .insert("content-type".into(), "application/json".into());
        let too_long = route_request(&too_long, &state).await.unwrap_err();
        assert!(too_long.downcast_ref::<UiBadRequest>().is_some());

        let mut missing_description = make_request(
            "PATCH",
            &format!("/api/v1/jobs/{}/description", uuid::Uuid::new_v4()),
            r#"{"description":"Missing","expected_revision":0}"#,
        );
        missing_description
            .headers
            .insert("content-type".into(), "application/json".into());
        let missing_description = route_request(&missing_description, &state)
            .await
            .unwrap_err();
        assert!(missing_description.downcast_ref::<UiNotFound>().is_some());
        assert_eq!(
            Store::open(&paths.database)
                .unwrap()
                .list_jobs(None)
                .unwrap()
                .len(),
            1
        );

        let mut missing_type = make_request("POST", "/api/v1/jobs", &body);
        assert!(
            route_request(&missing_type, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiUnsupportedMediaType>()
                .is_some()
        );
        missing_type
            .headers
            .insert("content-type".into(), "text/plain".into());
        assert!(
            route_request(&missing_type, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiUnsupportedMediaType>()
                .is_some()
        );

        let mut foreign_origin = make_request("GET", "/api/v1/fs/roots", "");
        foreign_origin
            .headers
            .insert("host".into(), "localhost".into());
        foreign_origin
            .headers
            .insert("origin".into(), "http://evil.example".into());
        assert!(
            route_request(&foreign_origin, &state)
                .await
                .unwrap_err()
                .downcast_ref::<UiForbidden>()
                .is_some()
        );
    }

    fn percent_encode_for_test(path: &Path) -> String {
        path.to_string_lossy()
            .bytes()
            .flat_map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._~/\\:".contains(&byte) {
                    vec![char::from(byte)]
                } else {
                    format!("%{byte:02X}").chars().collect()
                }
            })
            .collect()
    }

    #[test]
    fn large_logs_keep_only_the_latest_bounded_chunk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout.log");
        let mut contents = vec![b'a'; MAX_LOG_BYTES + 17];
        contents.extend_from_slice(b"tail");
        std::fs::write(&path, contents).unwrap();

        let (text, available, truncated) = read_log(&path).unwrap();
        assert!(available);
        assert!(truncated);
        assert_eq!(text.len(), MAX_LOG_BYTES);
        assert!(text.ends_with("tail"));
    }

    #[test]
    fn query_parser_decodes_filter_values() {
        let query = parse_query("user=alice%20team&state=running&empty");
        assert_eq!(query.get("user"), Some(&"alice team".to_owned()));
        assert_eq!(query.get("state"), Some(&"running".to_owned()));
        assert_eq!(query.get("empty"), Some(&"".to_owned()));
    }

    #[test]
    fn malformed_percent_encoding_is_ignored() {
        assert_eq!(percent_decode("ok%20value"), Some("ok value".into()));
        assert_eq!(percent_decode("bad%"), None);
        assert_eq!(parse_query("user=bad%"), std::collections::HashMap::new());
    }

    #[test]
    fn targets_split_path_and_query() {
        assert_eq!(
            split_target("/api/v1/jobs?state=queued"),
            ("/api/v1/jobs", "state=queued")
        );
        assert_eq!(split_target("/"), ("/", ""));
    }

    #[test]
    fn token_comparison_checks_length_and_content() {
        assert!(constant_time_equal(b"token", b"token"));
        assert!(!constant_time_equal(b"token", b"tokens"));
        assert!(!constant_time_equal(b"token", b"Token"));
    }

    #[test]
    fn lan_routes_require_the_exact_bearer_token() {
        let state = UiServerState {
            paths: test_paths(Path::new(".")),
            metadata: UiMetadata {
                pid: 7,
                host: "0.0.0.0".parse().unwrap(),
                port: 8765,
                auth_required: true,
            },
            token: Some("secret-token".into()),
            shutdown: Arc::new(Notify::new()),
            stopping: Arc::new(AtomicBool::new(false)),
        };
        let mut request = HttpRequest {
            method: "GET".into(),
            target: "/api/v1/status".into(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        assert!(!authorized(&request, &state));
        request
            .headers
            .insert("authorization".into(), "Bearer wrong".into());
        assert!(!authorized(&request, &state));
        request
            .headers
            .insert("authorization".into(), "Bearer secret-token".into());
        assert!(authorized(&request, &state));
    }

    #[tokio::test]
    async fn loopback_server_serves_embedded_shell_and_read_api() {
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
        let index = raw_request(
            metadata.port,
            "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(index.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&index).contains("Queue control center"));

        let config = raw_request(
            metadata.port,
            "GET /api/v1/ui/config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(config.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&config).contains("auth_required"));

        let status = raw_request(
            metadata.port,
            "GET /api/v1/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(status.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&status).contains("scheduler"));

        let draft_for_logs = Store::open(&paths.database)
            .unwrap()
            .create_job(NewJob {
                name: "logs-draft".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "draft".into()],
            })
            .unwrap();
        let log_dir = paths.runs.join(draft_for_logs.to_string());
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(log_dir.join("stdout.log"), b"draft output\n").unwrap();
        let logs = raw_request(
            metadata.port,
            &format!(
                "GET /api/v1/jobs/{draft_for_logs}/logs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert!(logs.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&logs).contains("draft output"));
        assert!(String::from_utf8_lossy(&logs).contains("DRAFT"));

        let terminal_for_clean = Store::open(&paths.database)
            .unwrap()
            .create_job(NewJob {
                name: "clean-terminal".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "clean".into()],
            })
            .unwrap();
        Store::open(&paths.database)
            .unwrap()
            .cancel_not_started(terminal_for_clean)
            .unwrap();
        let terminal_run_dir = paths.runs.join(terminal_for_clean.to_string());
        std::fs::create_dir_all(&terminal_run_dir).unwrap();
        std::fs::write(terminal_run_dir.join("stdout.log"), b"finished\n").unwrap();
        let cleaned = raw_request(
            metadata.port,
            "POST /api/v1/clean HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(cleaned.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&cleaned).contains(r#""removed":1"#));
        assert!(!terminal_run_dir.exists());
        assert!(matches!(
            Store::open(&paths.database)
                .unwrap()
                .get_job(terminal_for_clean),
            Err(crate::StoreError::NotFound { .. })
        ));

        let configuration = raw_request(
            metadata.port,
            "GET /api/v1/config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(configuration.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&configuration).contains("config_path"));
        assert!(String::from_utf8_lossy(&configuration).contains("timezones"));

        let timezone = raw_request(
            metadata.port,
            &json_request("PUT", "/api/v1/config/timezone", r#"{"value":"UTC"}"#),
        )
        .await;
        assert!(timezone.starts_with(b"HTTP/1.1 200 OK"));
        let jobs_after_timezone = raw_request(
            metadata.port,
            "GET /api/v1/jobs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(jobs_after_timezone.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&jobs_after_timezone).contains("UTC"));
        let invalid_timezone = raw_request(
            metadata.port,
            &json_request(
                "PUT",
                "/api/v1/config/timezone",
                r#"{"value":"Not/ARealTimezone"}"#,
            ),
        )
        .await;
        assert!(invalid_timezone.starts_with(b"HTTP/1.1 400 Bad Request"));

        let snapshot = raw_request(
            metadata.port,
            "POST /api/v1/config/snapshot HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(snapshot.starts_with(b"HTTP/1.1 200 OK"));
        let snapshot_body = response_body(&snapshot);
        let snapshot_json: serde_json::Value = serde_json::from_slice(snapshot_body).unwrap();
        let snapshot_path = snapshot_json["snapshots"][0]["path"]
            .as_str()
            .unwrap()
            .to_owned();

        let changed_timezone = raw_request(
            metadata.port,
            &json_request(
                "PUT",
                "/api/v1/config/timezone",
                r#"{"value":"Asia/Tokyo"}"#,
            ),
        )
        .await;
        assert!(changed_timezone.starts_with(b"HTTP/1.1 200 OK"));
        let restore_body =
            serde_json::to_string(&serde_json::json!({"path": snapshot_path})).unwrap();
        let restored = raw_request(
            metadata.port,
            &json_request("POST", "/api/v1/config/restore", &restore_body),
        )
        .await;
        assert!(restored.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&restored).contains(r#""timezone":"UTC""#));

        let store = Store::open(&paths.database).unwrap();
        let first = store
            .create_job(NewJob {
                name: "queue-first".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "first".into()],
            })
            .unwrap();
        let second = store
            .create_job(NewJob {
                name: "queue-second".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "second".into()],
            })
            .unwrap();
        store.commit_job(first).unwrap();
        store.commit_job(second).unwrap();

        let locked = raw_request(
            metadata.port,
            "POST /api/v1/queue/lock HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(locked.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&locked).contains(r#""locked":true"#));
        let moved = raw_request(
            metadata.port,
            &json_request(
                "POST",
                &format!("/api/v1/queue/{second}/move"),
                r#"{"target_order":1}"#,
            ),
        )
        .await;
        assert!(moved.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&moved).contains("queue-second"));
        let unlocked = raw_request(
            metadata.port,
            "POST /api/v1/queue/unlock HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(unlocked.starts_with(b"HTTP/1.1 200 OK"));
        assert!(String::from_utf8_lossy(&unlocked).contains(r#""locked":false"#));
        let unlocked_move = raw_request(
            metadata.port,
            &json_request(
                "POST",
                &format!("/api/v1/queue/{second}/move"),
                r#"{"target_order":1}"#,
            ),
        )
        .await;
        assert!(unlocked_move.starts_with(b"HTTP/1.1 409 Conflict"));

        let shutdown = raw_request(
            metadata.port,
            "POST /__stoker/shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(shutdown.starts_with(b"HTTP/1.1 200 OK"));
        server.await.unwrap();
        assert!(read_metadata(&paths).unwrap().is_none());
    }

    fn make_request(method: &str, target: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.into(),
            target: target.into(),
            headers: HashMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn state_for(paths: &StokerPaths, auth_required: bool, token: Option<&str>) -> UiServerState {
        UiServerState {
            paths: paths.clone(),
            metadata: UiMetadata {
                pid: 7,
                host: if auth_required {
                    "0.0.0.0".parse().unwrap()
                } else {
                    "127.0.0.1".parse().unwrap()
                },
                port: 8765,
                auth_required,
            },
            token: token.map(str::to_owned),
            shutdown: Arc::new(Notify::new()),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn one_shot_request(state: UiServerState, request: &str) -> Vec<u8> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, state).await.unwrap();
        });
        let response = raw_request(port, request).await;
        server.await.unwrap();
        response
    }

    async fn wait_for_metadata(paths: &StokerPaths) -> super::UiMetadata {
        for _ in 0..50 {
            if let Some(metadata) = read_metadata(paths).unwrap() {
                return metadata;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("UI metadata was not written");
    }

    async fn raw_request(port: u16, request: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    }

    fn json_request(method: &str, path: &str, body: &str) -> String {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn response_body(response: &[u8]) -> &[u8] {
        response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| &response[index + 4..])
            .unwrap()
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
