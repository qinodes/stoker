use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, patch, post, put};

use super::assets;
use super::auth;
use super::handlers::{configuration, filesystem, jobs, policy, queue, status, system};
use super::state::ApiState;

pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub(super) fn build_router(state: ApiState) -> Router {
    let api_routes = Router::new()
        .route("/status", get(status::status))
        .route("/jobs", get(jobs::list).post(jobs::create))
        .route("/jobs/{id}", get(jobs::detail))
        .route("/jobs/{id}/description", patch(jobs::update_description))
        .route("/jobs/{id}/commit", post(jobs::commit))
        .route("/jobs/{id}/cancel", post(jobs::cancel))
        .route("/jobs/{id}/logs", get(jobs::logs))
        .route("/clean", post(jobs::clean))
        .route("/queue", get(queue::get))
        .route("/queue/lock", post(queue::lock))
        .route("/queue/unlock", post(queue::unlock))
        .route("/queue/{id}/move", post(queue::move_job))
        .route("/policy", get(policy::get))
        .route("/policy/{key}", put(policy::set).delete(policy::unset))
        .route("/fs/roots", get(filesystem::roots))
        .route("/fs/directories", get(filesystem::directories))
        .route("/config", get(configuration::get))
        .route("/config/snapshots", get(configuration::get))
        .route("/config/snapshot", post(configuration::create_snapshot))
        .route(
            "/config/timezone",
            put(configuration::set_timezone).delete(configuration::unset_timezone),
        )
        .route("/config/restore", post(configuration::restore_snapshot))
        .fallback(system::api_not_found)
        .method_not_allowed_fallback(system::method_not_allowed);

    let api = Router::new()
        .route("/ui/config", get(system::ui_config))
        .merge(api_routes)
        .fallback(system::api_not_found)
        .method_not_allowed_fallback(system::method_not_allowed)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::validate_source,
        ));

    Router::new()
        .route("/", get(assets::index))
        .route("/index.html", get(assets::index))
        .route("/app.js", get(assets::app_js))
        .route("/styles.css", get(assets::styles_css))
        .route("/assets/logo.svg", get(assets::logo_svg))
        .route("/assets/logo-mark.png", get(assets::logo_mark_png))
        .route("/__stoker/shutdown", post(system::shutdown))
        .nest("/api/v1", api)
        .fallback(assets::not_found)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(no_store))
        .with_state(state)
}

async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde::Deserialize;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::ServiceClient;
    use crate::config::StokerPaths;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct HttpRouteFixture {
        name: String,
        method: String,
        target: String,
        #[serde(default)]
        json_body: Option<String>,
        #[serde(default)]
        raw_body: Option<String>,
        #[serde(default)]
        origin: Option<String>,
        expected_status: u16,
        expected_fields: Vec<String>,
    }

    #[tokio::test]
    async fn router_matches_literal_v1_route_status_and_shape_fixtures_without_tcp() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let fixtures: Vec<HttpRouteFixture> =
            serde_json::from_str(include_str!("../../tests/fixtures/http/v1/routes.json")).unwrap();

        for fixture in fixtures {
            let response = build_router(state_for(&paths))
                .oneshot(fixture_request(&fixture, directory.path()))
                .await
                .unwrap();
            assert_eq!(
                response.status().as_u16(),
                fixture.expected_status,
                "{}",
                fixture.name
            );
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let body = to_bytes(response.into_body(), MAX_REQUEST_BYTES)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&body)
                .unwrap_or_else(|error| panic!("{} returned invalid JSON: {error}", fixture.name));
            for field in fixture.expected_fields {
                assert!(
                    value.get(&field).is_some(),
                    "{} omitted {field}",
                    fixture.name
                );
            }
            if fixture.expected_status >= 400 {
                for field in ["error", "code", "message", "details"] {
                    assert!(
                        value.get(field).is_some(),
                        "{} omitted {field}",
                        fixture.name
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn body_limit_and_json_rejections_use_typed_error_contract() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let response = build_router(state_for(&paths))
            .oneshot(
                Request::post("/api/v1/jobs")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(vec![b'x'; MAX_REQUEST_BYTES + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), MAX_REQUEST_BYTES)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "payload_too_large");

        let response = build_router(state_for(&paths))
            .oneshot(
                Request::post("/api/v1/jobs")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn embedded_assets_and_react_entrypoint_load_with_correct_types() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        for (path, content_type) in [
            ("/", "text/html; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/styles.css", "text/css; charset=utf-8"),
            ("/assets/logo-mark.png", "image/png"),
        ] {
            let response = build_router(state_for(&paths))
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        for path in ["/modules/controller.js", "/styles/tokens.css"] {
            let response = build_router(state_for(&paths))
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn job_detail_description_logs_and_filesystem_flow_through_axum_router() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths);
        let created = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "POST",
                    "/api/v1/jobs",
                    serde_json::json!({
                        "user": "alice",
                        "name": "router-flow",
                        "cwd": directory.path(),
                        "command": "echo router",
                        "description": null
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        let id = created["job"]["id"].as_str().unwrap();

        let detail = json_response(
            build_router(state.clone())
                .oneshot(
                    Request::get(format!("/api/v1/jobs/{id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(detail["job"]["name"], "router-flow");
        assert_eq!(detail["working_directory_status"], "planned");

        let updated = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "PATCH",
                    &format!("/api/v1/jobs/{id}/description"),
                    serde_json::json!({
                        "description": "updated through HTTP",
                        "expected_revision": 0
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(updated["job"]["description_revision"], 1);

        let run_dir = paths.runs.join(id);
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("stdout.log"), b"router output\n").unwrap();
        let logs = json_response(
            build_router(state.clone())
                .oneshot(
                    Request::get(format!("/api/v1/jobs/{id}/logs"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(logs["stdout"], "router output\n");
        assert_eq!(logs["job"]["description"], "updated through HTTP");

        let response = build_router(state.clone())
            .oneshot(
                Request::get("/api/v1/jobs/not-a-uuid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_response(response).await["code"], "invalid_input");

        let directories = json_response(
            build_router(state)
                .oneshot(
                    Request::get(format!(
                        "/api/v1/fs/directories?path={}",
                        percent_encode(directory.path().to_string_lossy().as_bytes())
                    ))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert!(directories["directories"].is_array());
    }

    #[tokio::test]
    async fn offline_queue_lock_move_and_unlock_keep_application_policy() {
        use crate::domain::NewJob;

        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths);
        let first = state
            .store
            .create_job(NewJob {
                name: "first".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().into(),
                command: vec!["echo".into(), "first".into()],
            })
            .unwrap();
        let second = state
            .store
            .create_job(NewJob {
                name: "second".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().into(),
                command: vec!["echo".into(), "second".into()],
            })
            .unwrap();
        state.store.commit_job(first).unwrap();
        state.store.commit_job(second).unwrap();

        let locked = json_response(
            build_router(state.clone())
                .oneshot(
                    Request::post("/api/v1/queue/lock")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(locked["locked"], true);
        let moved = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "POST",
                    &format!("/api/v1/queue/{second}/move"),
                    serde_json::json!({"target_order": 1}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(moved["jobs"][0]["id"], second.to_string());

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/unlock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let conflict = build_router(state)
            .oneshot(json_request(
                "POST",
                &format!("/api/v1/queue/{second}/move"),
                serde_json::json!({"target_order": 1}),
            ))
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        assert_eq!(json_response(conflict).await["code"], "conflict");
    }

    #[tokio::test]
    async fn timezone_snapshot_and_restore_round_trip_through_handlers() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths);
        let set = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "PUT",
                    "/api/v1/config/timezone",
                    serde_json::json!({"value": "UTC"}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(set["config"]["timezone"], "UTC");
        let snapshot = json_response(
            build_router(state.clone())
                .oneshot(
                    Request::post("/api/v1/config/snapshot")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        let path = snapshot["snapshots"][0]["path"].as_str().unwrap();
        build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/config/timezone",
                serde_json::json!({"value": "Asia/Tokyo"}),
            ))
            .await
            .unwrap();
        let restored = json_response(
            build_router(state)
                .oneshot(json_request(
                    "POST",
                    "/api/v1/config/restore",
                    serde_json::json!({"path": path}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(restored["config"]["timezone"], "UTC");
    }

    #[tokio::test]
    async fn policy_round_trip_exposes_defaults_and_enforces_queue_gate() {
        use crate::domain::NewJob;

        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths);

        let initial = json_response(
            build_router(state.clone())
                .oneshot(Request::get("/api/v1/policy").body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(initial["queue_locked"], false);
        assert_eq!(initial["can_update"], false);
        assert_eq!(initial["log"]["max_bytes_per_job"], 64);
        assert_eq!(
            initial["defaults"]["runtime"]["max_runtime_ms"],
            Value::Null
        );
        assert_eq!(initial["units"]["log"]["max_bytes_per_job"], "MB");
        assert_eq!(initial["units"]["log"]["retention_jobs"], "jobs");

        let unlocked = build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/log-max-bytes-per-job",
                serde_json::json!({"value": 8}),
            ))
            .await
            .unwrap();
        assert_eq!(unlocked.status(), StatusCode::CONFLICT);
        let error = json_response(unlocked).await;
        assert_eq!(error["code"], "conflict");
        assert_eq!(error["details"]["queue_locked"], false);

        let invalid = build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/not-a-policy",
                serde_json::json!({"value": 1}),
            ))
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_response(invalid).await["code"], "invalid_input");

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/lock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let updated = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "PUT",
                    "/api/v1/policy/log-max-bytes-per-job",
                    serde_json::json!({"value": 8}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(updated["log"]["max_bytes_per_job"], 8);
        assert_eq!(updated["can_update"], true);

        for (key, value, field, expected) in [
            ("log-segment-bytes", 1, "segment_bytes", 1),
            ("log-max-bytes-total", 16, "max_bytes_total", 16),
            ("log-retention-jobs", 0, "retention_jobs", 0),
            ("log-disk-reserve-bytes", 1, "disk_reserve_bytes", 1),
        ] {
            let response = json_response(
                build_router(state.clone())
                    .oneshot(json_request(
                        "PUT",
                        &format!("/api/v1/policy/{key}"),
                        serde_json::json!({"value": value}),
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(response["log"][field], expected);
        }

        let invalid_type = build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/log-max-bytes-per-job",
                serde_json::json!({"value": "8"}),
            ))
            .await
            .unwrap();
        assert_eq!(invalid_type.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_response(invalid_type).await["code"], "invalid_input");

        let overflow = build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/log-max-bytes-per-job",
                serde_json::json!({"value": u64::MAX}),
            ))
            .await
            .unwrap();
        assert_eq!(overflow.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_response(overflow).await["code"], "invalid_input");

        for key in [
            "log-max-bytes-total",
            "log-max-bytes-per-job",
            "log-segment-bytes",
            "log-retention-jobs",
            "log-disk-reserve-bytes",
        ] {
            let response = build_router(state.clone())
                .oneshot(
                    Request::delete(format!("/api/v1/policy/{key}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if response.status() != StatusCode::OK {
                panic!("reset {key} failed: {}", json_response(response).await);
            }
        }

        let runtime = json_response(
            build_router(state.clone())
                .oneshot(json_request(
                    "PUT",
                    "/api/v1/policy/max-runtime-ms",
                    serde_json::json!({"value": 1_000}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(runtime["runtime"]["max_runtime_ms"], 1_000);

        for (key, value, field) in [
            ("termination-grace-ms", 750, "termination_grace_ms"),
            ("startup-timeout-ms", 1_000, "startup_timeout_ms"),
        ] {
            let response = json_response(
                build_router(state.clone())
                    .oneshot(json_request(
                        "PUT",
                        &format!("/api/v1/policy/{key}"),
                        serde_json::json!({"value": value}),
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(response["runtime"][field], value);
        }

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/unlock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let runtime_unlocked = build_router(state.clone())
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/termination-grace-ms",
                serde_json::json!({"value": 750}),
            ))
            .await
            .unwrap();
        assert_eq!(runtime_unlocked.status(), StatusCode::CONFLICT);
        assert_eq!(json_response(runtime_unlocked).await["code"], "conflict");
        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/lock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let reset = json_response(
            build_router(state.clone())
                .oneshot(
                    Request::delete("/api/v1/policy/max-runtime-ms")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(reset["runtime"]["max_runtime_ms"], Value::Null);

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/unlock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = state
            .store
            .create_job(NewJob {
                name: "active-policy-job".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().into(),
                command: vec!["echo".into(), "active".into()],
            })
            .unwrap();
        state.store.commit_job(id).unwrap();
        state.store.claim_next().unwrap();
        state.store.lock_queue().unwrap();
        let active_policy = json_response(
            build_router(state.clone())
                .oneshot(Request::get("/api/v1/policy").body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(active_policy["can_update"], false);
        assert_eq!(active_policy["active_jobs"][0]["id"], id.to_string());
        assert_eq!(active_policy["active_jobs"][0]["state"], "STARTING");
        assert!(
            active_policy["blocked_reason"]
                .as_str()
                .unwrap()
                .contains("STARTING")
        );

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/unlock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unlocked_with_active = json_response(
            build_router(state.clone())
                .oneshot(Request::get("/api/v1/policy").body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(unlocked_with_active["can_update"], false);
        assert!(
            unlocked_with_active["blocked_reason"]
                .as_str()
                .unwrap()
                .contains("Lock the queue and wait")
        );

        build_router(state.clone())
            .oneshot(
                Request::post("/api/v1/queue/lock")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let active = build_router(state)
            .oneshot(json_request(
                "PUT",
                "/api/v1/policy/termination-grace-ms",
                serde_json::json!({"value": 750}),
            ))
            .await
            .unwrap();
        assert_eq!(active.status(), StatusCode::CONFLICT);
        let active_error = json_response(active).await;
        assert_eq!(active_error["details"]["state"], "STARTING");
        assert_eq!(active_error["details"]["active_job"], id.to_string());
    }

    fn fixture_request(fixture: &HttpRouteFixture, fixture_root: &Path) -> Request<Body> {
        let body = fixture
            .json_body
            .as_deref()
            .map(|body| {
                let mut value: Value = serde_json::from_str(body).unwrap();
                if value.get("cwd").and_then(Value::as_str) == Some("$FIXTURE_ROOT") {
                    value["cwd"] = Value::String(fixture_root.to_string_lossy().into_owned());
                }
                serde_json::to_string(&value).unwrap()
            })
            .or_else(|| fixture.raw_body.clone())
            .unwrap_or_default();
        let mut request = Request::builder()
            .method(fixture.method.as_str())
            .uri(&fixture.target)
            .header(header::HOST, "localhost");
        if fixture.json_body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(value) = &fixture.origin {
            request = request.header(header::ORIGIN, value);
        }
        request.body(Body::from(body)).unwrap()
    }

    fn json_request(method: &str, path: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    async fn json_response(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), MAX_REQUEST_BYTES)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn percent_encode(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(byte) {
                    char::from(*byte).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect()
    }

    pub(super) fn state_for(paths: &StokerPaths) -> ApiState {
        ApiState::new(
            paths.clone(),
            crate::Store::open(&paths.database).unwrap(),
            ServiceClient::new(paths.clone()),
        )
    }

    pub(super) fn test_paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }
}
