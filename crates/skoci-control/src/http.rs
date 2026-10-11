use axum::async_trait;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::auth::{self, Subject};
use crate::state::AppState;

pub fn api_reply<T: Serialize>(res: SkociResult<T>) -> Response {
    match res {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse::ok(data, "Success")),
        )
            .into_response(),
        Err(err) => {
            let status =
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(ApiResponse::<()>::err(err))).into_response()
        }
    }
}

pub fn api_accepted<T: Serialize>(data: T, summary: &str) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse::ok(data, summary)),
    )
        .into_response()
}

pub struct AuthSubject(pub Subject);

#[async_trait]
impl FromRequestParts<AppState> for AuthSubject {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer_ip = extract_ip(parts);

        // 1. Try Bearer token
        if let Some(auth_header) = parts.headers.get("authorization").and_then(|v| v.to_str().ok()) {
            let token = if let Some(stripped) = auth_header.strip_prefix("Bearer ") {
                stripped.trim()
            } else if let Some(stripped) = auth_header.strip_prefix("bearer ") {
                stripped.trim()
            } else {
                auth_header.trim()
            };

            match auth::authenticate_token(&state.db, &state.config.master_token, token) {
                Ok(subject) => return Ok(AuthSubject(subject)),
                Err(err) => {
                    let is_blocked = state.rate_limiter.check(peer_ip).is_err();
                    state.rate_limiter.record_failure(peer_ip);
                    let status = if is_blocked {
                        StatusCode::TOO_MANY_REQUESTS
                    } else {
                        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED)
                    };
                    return Err((status, Json(ApiResponse::<()>::err(err))).into_response());
                }
            }
        }

        // 2. Try Cookie session
        if let Some(cookie_header) = parts.headers.get("cookie").and_then(|v| v.to_str().ok()) {
            if let Some(session_id) = extract_cookie(cookie_header, "skoci_session") {
                if state.sessions.validate_session(&session_id) {
                    if parts.method != axum::http::Method::GET
                        && parts.method != axum::http::Method::HEAD
                        && parts.method != axum::http::Method::OPTIONS
                    {
                        if let Some(origin_str) = parts.headers.get("origin").and_then(|v| v.to_str().ok()) {
                            let origin_host = if let Some(stripped) = origin_str.strip_prefix("https://") {
                                stripped
                            } else if let Some(stripped) = origin_str.strip_prefix("http://") {
                                stripped
                            } else {
                                origin_str
                            };
                            let origin_host = origin_host.trim_end_matches('/');
                            let host_str = parts.headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
                            if !origin_host.eq_ignore_ascii_case(host_str) {
                                let err = SkociError::forbidden(
                                    format!("Origin '{origin_str}' does not match Host '{host_str}'"),
                                    "Cross-origin requests with dashboard session are not allowed",
                                );
                                let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::FORBIDDEN);
                                return Err((status, Json(ApiResponse::<()>::err(err))).into_response());
                            }
                        }

                        let csrf = parts
                            .headers
                            .get("x-skoci-dashboard")
                            .and_then(|v| v.to_str().ok());
                        if csrf != Some("1") {
                            let err = SkociError::forbidden(
                                "Header X-Skoci-Dashboard: 1 is required for dashboard sessions",
                                "Add header 'X-Skoci-Dashboard: 1' to this request",
                            );
                            let status = StatusCode::from_u16(err.http_status())
                                .unwrap_or(StatusCode::FORBIDDEN);
                            return Err((status, Json(ApiResponse::<()>::err(err))).into_response());
                        }
                    }
                    return Ok(AuthSubject(Subject::Admin));
                } else {
                    state.rate_limiter.record_failure(peer_ip);
                    let err = SkociError::unauthorized(
                        "Dashboard session is invalid or expired",
                    );
                    let status = if state.rate_limiter.check(peer_ip).is_err() {
                        StatusCode::TOO_MANY_REQUESTS
                    } else {
                        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED)
                    };
                    return Err((status, Json(ApiResponse::<()>::err(err))).into_response());
                }
            }
        }

        // Neither credential provided: check rate limit first, then 401
        if let Err(err) = state.rate_limiter.check(peer_ip) {
            let status = StatusCode::from_u16(err.http_status())
                .unwrap_or(StatusCode::TOO_MANY_REQUESTS);
            return Err((status, Json(ApiResponse::<()>::err(err))).into_response());
        }

        let err = SkociError::unauthorized(
            "Authentication required. Provide a Bearer token or log in to the dashboard",
        );
        let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED);
        Err((status, Json(ApiResponse::<()>::err(err))).into_response())
    }
}

fn extract_ip(parts: &Parts) -> IpAddr {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    client_ip(peer, &parts.headers)
}

/// The control plane only listens on loopback, so remote callers arrive through the Skoci proxy,
/// which appends the real peer address as the last `X-Forwarded-For` entry. Earlier entries are
/// client-supplied and never trusted.
pub fn client_ip(peer: Option<SocketAddr>, headers: &axum::http::HeaderMap) -> IpAddr {
    let loopback = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    match peer {
        Some(addr) if !addr.ip().is_loopback() => addr.ip(),
        _ => headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|last| last.trim().parse::<IpAddr>().ok())
            .or(peer.map(|a| a.ip()))
            .unwrap_or(loopback),
    }
}

fn extract_cookie(cookie_header: &str, name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}
