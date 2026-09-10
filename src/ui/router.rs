use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, patch, post, put};

use super::assets;
use super::auth;
use super::handlers::{configuration, filesystem, jobs, queue, status, system};
use super::state::ApiState;

pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub(super) fn build_router(state: ApiState) -> Router {
    let protected = Router::new()
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
        .method_not_allowed_fallback(system::method_not_allowed)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_authorization,
        ));

    let api = Router::new()
        .route("/ui/config", get(system::ui_config))
        .merge(protected)
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
        .route("/modules/{*path}", get(assets::module))
        .route("/styles/{*path}", get(assets::style))
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
    use crate::ui::UiMetadata;

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
        auth_required: bool,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        authorization: Option<String>,
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
            let response = build_router(state_for(
                &paths,
                fixture.auth_required,
                fixture.token.as_deref(),
            ))
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
        let response = build_router(state_for(&paths, false, None))
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

        let response = build_router(state_for(&paths, false, None))
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
    async fn embedded_assets_and_module_entrypoint_load_with_correct_types() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        for (path, content_type) in [
            ("/", "text/html; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/styles.css", "text/css; charset=utf-8"),
            ("/assets/logo-mark.png", "image/png"),
        ] {
            let response = build_router(state_for(&paths, false, None))
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }

    #[tokio::test]
    async fn job_detail_description_logs_and_filesystem_flow_through_axum_router() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        paths.ensure().unwrap();
        let state = state_for(&paths, false, None);
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
        let state = state_for(&paths, false, None);
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
        let state = state_for(&paths, false, None);
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
        if let Some(value) = &fixture.authorization {
            request = request.header(header::AUTHORIZATION, value);
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

    pub(super) fn state_for(
        paths: &StokerPaths,
        auth_required: bool,
        token: Option<&str>,
    ) -> ApiState {
        ApiState::new(
            paths.clone(),
            crate::Store::open(&paths.database).unwrap(),
            ServiceClient::new(paths.clone()),
            UiMetadata {
                pid: 7,
                host: if auth_required {
                    "0.0.0.0".parse().unwrap()
                } else {
                    "127.0.0.1".parse().unwrap()
                },
                port: 8765,
                auth_required,
            },
            token.map(str::to_owned),
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
