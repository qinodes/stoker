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

pub(super) async fn require_authorization(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !authorized(request.headers(), &state) {
        return Err(ApiError::unauthorized());
    }
    Ok(next.run(request).await)
}

pub(super) fn authorized(headers: &HeaderMap, state: &ApiState) -> bool {
    if !state.metadata.auth_required {
        return true;
    }
    let Some(expected) = state.token.as_deref() else {
        return false;
    };
    let Some(header) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(received) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_equal(expected.as_bytes(), received.as_bytes())
}

/// Browser origins must match the HTTP Host authority. Non-browser local
/// clients may omit Origin; bearer authentication still protects LAN mode.
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

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= (left.get(index).copied().unwrap_or(0)
            ^ right.get(index).copied().unwrap_or(0)) as usize;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceClient;
    use crate::config::StokerPaths;
    use std::path::Path;

    fn state(auth_required: bool, token: Option<&str>) -> ApiState {
        let paths = StokerPaths {
            root: Path::new(".").into(),
            database: Path::new(":memory:").into(),
            runs: Path::new("runs").into(),
            lock: Path::new("lock").into(),
            endpoint: Path::new("endpoint").into(),
        };
        ApiState::new(
            paths.clone(),
            crate::Store::open(":memory:").unwrap(),
            ServiceClient::new(paths),
            crate::ui::UiMetadata {
                pid: 1,
                host: "0.0.0.0".parse().unwrap(),
                port: 8765,
                auth_required,
            },
            token.map(str::to_owned),
        )
    }

    #[test]
    fn lan_authorization_requires_exact_constant_time_bearer_token() {
        let state = state(true, Some("secret"));
        let mut headers = HeaderMap::new();
        assert!(!authorized(&headers, &state));
        headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(!authorized(&headers, &state));
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(authorized(&headers, &state));
        assert!(authorized(&HeaderMap::new(), &self::state(false, None)));
    }

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
