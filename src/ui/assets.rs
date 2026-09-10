use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;

// The React/TypeScript production bundle is checked in under web/dist so a
// normal `cargo install` does not require Node.js. `npm run build` refreshes
// these deterministic assets during development and CI.
const INDEX_HTML: &[u8] = include_bytes!("../../web/dist/index.html");
const APP_JS: &[u8] = include_bytes!("../../web/dist/app.js");
const STYLES_CSS: &[u8] = include_bytes!("../../web/dist/styles.css");
const LOGO_SVG: &[u8] = include_bytes!("../../assets/logo.svg");
const LOGO_MARK_PNG: &[u8] = include_bytes!("../../assets/logo-mark.png");

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

pub(super) async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "UI asset not found")
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
        ] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        assert!(LOGO_MARK_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
