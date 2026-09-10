use axum::body::Body;
use axum::extract::Path;
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;

const INDEX_HTML: &[u8] = include_bytes!("../../web/index.html");
const APP_JS: &[u8] = include_bytes!("../../web/app.js");
const STYLES_CSS: &[u8] = include_bytes!("../../web/styles.css");
const LOGO_SVG: &[u8] = include_bytes!("../../assets/logo.svg");
const LOGO_MARK_PNG: &[u8] = include_bytes!("../../assets/logo-mark.png");
const API_CLIENT_JS: &[u8] = include_bytes!("../../web/modules/api-client.js");
const COMPONENTS_JS: &[u8] = include_bytes!("../../web/modules/components.js");
const CONTROLLER_JS: &[u8] = include_bytes!("../../web/modules/controller.js");
const FORMATTERS_JS: &[u8] = include_bytes!("../../web/modules/formatters.js");
const STATE_JS: &[u8] = include_bytes!("../../web/modules/state.js");
const CONFIGURATION_VIEW_JS: &[u8] = include_bytes!("../../web/modules/views/configuration.js");
const JOBS_VIEW_JS: &[u8] = include_bytes!("../../web/modules/views/jobs.js");
const LOGS_VIEW_JS: &[u8] = include_bytes!("../../web/modules/views/logs.js");
const OVERVIEW_VIEW_JS: &[u8] = include_bytes!("../../web/modules/views/overview.js");
const QUEUE_VIEW_JS: &[u8] = include_bytes!("../../web/modules/views/queue.js");
const TOKENS_CSS: &[u8] = include_bytes!("../../web/styles/tokens.css");
const BASE_CSS: &[u8] = include_bytes!("../../web/styles/base.css");
const LAYOUT_CSS: &[u8] = include_bytes!("../../web/styles/layout.css");
const COMPONENTS_CSS: &[u8] = include_bytes!("../../web/styles/components.css");
const OVERVIEW_CSS: &[u8] = include_bytes!("../../web/styles/overview.css");
const DATA_VIEWS_CSS: &[u8] = include_bytes!("../../web/styles/data-views.css");
const JOB_DIALOGS_CSS: &[u8] = include_bytes!("../../web/styles/job-dialogs.css");
const RESPONSIVE_CSS: &[u8] = include_bytes!("../../web/styles/responsive.css");

pub(super) async fn index() -> Response<Body> {
    asset_response("text/html; charset=utf-8", INDEX_HTML)
}

pub(super) async fn app_js() -> Response<Body> {
    asset_response("text/javascript; charset=utf-8", APP_JS)
}

pub(super) async fn styles_css() -> Response<Body> {
    asset_response("text/css; charset=utf-8", STYLES_CSS)
}

pub(super) async fn logo_svg() -> Response<Body> {
    asset_response("image/svg+xml", LOGO_SVG)
}

pub(super) async fn logo_mark_png() -> Response<Body> {
    asset_response("image/png", LOGO_MARK_PNG)
}

pub(super) async fn module(Path(path): Path<String>) -> Response<Body> {
    let bytes = match path.as_str() {
        "api-client.js" => API_CLIENT_JS,
        "components.js" => COMPONENTS_JS,
        "controller.js" => CONTROLLER_JS,
        "formatters.js" => FORMATTERS_JS,
        "state.js" => STATE_JS,
        "views/configuration.js" => CONFIGURATION_VIEW_JS,
        "views/jobs.js" => JOBS_VIEW_JS,
        "views/logs.js" => LOGS_VIEW_JS,
        "views/overview.js" => OVERVIEW_VIEW_JS,
        "views/queue.js" => QUEUE_VIEW_JS,
        _ => return not_found_response(),
    };
    asset_response("text/javascript; charset=utf-8", bytes)
}

pub(super) async fn style(Path(path): Path<String>) -> Response<Body> {
    let bytes = match path.as_str() {
        "tokens.css" => TOKENS_CSS,
        "base.css" => BASE_CSS,
        "layout.css" => LAYOUT_CSS,
        "components.css" => COMPONENTS_CSS,
        "overview.css" => OVERVIEW_CSS,
        "data-views.css" => DATA_VIEWS_CSS,
        "job-dialogs.css" => JOB_DIALOGS_CSS,
        "responsive.css" => RESPONSIVE_CSS,
        _ => return not_found_response(),
    };
    asset_response("text/css; charset=utf-8", bytes)
}

pub(super) async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "UI asset not found")
}

fn not_found_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("UI asset not found"))
        .expect("static error response is valid")
}

fn asset_response(content_type: &'static str, bytes: &'static [u8]) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .expect("static asset response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn embedded_assets_have_expected_types_and_signatures() {
        for (response, content_type) in [
            (index().await, "text/html; charset=utf-8"),
            (app_js().await, "text/javascript; charset=utf-8"),
            (styles_css().await, "text/css; charset=utf-8"),
            (logo_svg().await, "image/svg+xml"),
            (logo_mark_png().await, "image/png"),
            (
                module(Path("controller.js".into())).await,
                "text/javascript; charset=utf-8",
            ),
            (
                style(Path("tokens.css".into())).await,
                "text/css; charset=utf-8",
            ),
        ] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        assert!(LOGO_MARK_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
