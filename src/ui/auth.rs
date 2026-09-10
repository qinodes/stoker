use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use super::error::ApiError;
use super::state::ApiState;

pub(super) async fn validate_source(
    State(_state): State<ApiState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !request_source_allowed(request.headers()) {
        return Err(ApiError::forbidden("UI request origin is not allowed"));
    }
    Ok(next.run(request).await)
}

/// Browser origins must match the HTTP Host authority. Non-browser clients may
/// omit Origin. This prevents cross-origin browser requests while keeping the
/// trusted-network LAN mode free of an additional token prompt.
pub(super) fn request_source_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
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
    headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host.eq_ignore_ascii_case(authority))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_origin_must_match_host_and_http_scheme() {
        let mut headers = HeaderMap::new();
        assert!(request_source_allowed(&headers));
        headers.insert("host", "localhost:8765".parse().unwrap());
        for origin in [
            "null",
            "https://localhost:8765",
            "http://evil.test",
            "invalid",
        ] {
            headers.insert("origin", origin.parse().unwrap());
            assert!(!request_source_allowed(&headers), "{origin}");
        }
        headers.insert("origin", "http://localhost:8765".parse().unwrap());
        assert!(request_source_allowed(&headers));
    }
}
