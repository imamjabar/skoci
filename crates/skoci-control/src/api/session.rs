use axum::extract::{ConnectInfo, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::models::ApiResponse;
use skoci_api::requests::SessionRequest;
use skoci_api::responses::SessionResponse;

use crate::auth::constant_time_eq;
use crate::http::{api_reply, client_ip, AuthSubject};
use std::net::SocketAddr;
use crate::state::AppState;

pub async fn handle_create_session(
    State(state): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<SessionRequest>,
) -> Response {
    let ip = client_ip(peer.map(|ConnectInfo(a)| a), &headers);
    if let Err(err) = state.rate_limiter.check(ip) {
        let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
        return (status, Json(ApiResponse::<()>::err(err))).into_response();
    }
    let token = body.token.trim();
    if !constant_time_eq(token.as_bytes(), state.config.master_token.as_bytes()) {
        state.rate_limiter.record_failure(ip);
        let err = SkociError::unauthorized("Master token salah");
        let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED);
        return (status, Json(ApiResponse::<()>::err(err))).into_response();
    }

    let (session_id, expires_at) = state.sessions.create_session();

    let is_https = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false);

    let secure_flag = if is_https { "; Secure" } else { "" };
    let cookie_val = format!(
        "skoci_session={session_id}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200{secure_flag}"
    );

    let mut resp = api_reply(Ok(SessionResponse { expires_at }));
    if let Ok(hv) = cookie_val.parse() {
        resp.headers_mut().insert(SET_COOKIE, hv);
    }
    resp
}

pub async fn handle_delete_session(
    State(state): State<AppState>,
    AuthSubject(_subject): AuthSubject,
    headers: HeaderMap,
) -> Response {
    if let Some(cookie_hdr) = headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie_hdr.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k.trim() == "skoci_session" {
                    state.sessions.delete_session(v.trim());
                }
            }
        }
    }

    let cookie_val = "skoci_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0";
    let mut resp = api_reply(Ok(()));
    if let Ok(hv) = cookie_val.parse() {
        resp.headers_mut().insert(SET_COOKIE, hv);
    }
    resp
}
