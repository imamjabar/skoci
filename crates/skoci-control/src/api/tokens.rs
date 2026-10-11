use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use skoci_api::requests::TokenCreateRequest;
use skoci_api::responses::{
    TokenCreateResponse, TokenInfo, TokenListResponse, TokenRevokeResponse,
};

use crate::auth::generate_agent_token;
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_create_token(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<TokenCreateRequest>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    let (token, hash, id) = generate_agent_token(&body.app);
    let created_at = Utc::now();

    if let Err(e) = state.db.insert_agent_token(&hash, &body.app, &body.name) {
        return api_reply::<()>(Err(e));
    }

    let _ = state.db.record_audit(
        &subject.identifier(),
        "admin",
        "token_create",
        &format!("{}/{}", body.app, body.name),
        &format!("Token ID {id} created"),
    );

    api_reply(Ok(TokenCreateResponse {
        token,
        id,
        app: body.app,
        name: body.name,
        created_at,
    }))
}

pub async fn handle_list_tokens(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    match state.db.list_agent_tokens_for_app(&app) {
        Ok(records) => {
            let tokens = records
                .into_iter()
                .map(|r| {
                    let id = if r.token_hash.len() >= 12 {
                        r.token_hash[..12].to_string()
                    } else {
                        r.token_hash
                    };
                    TokenInfo {
                        id,
                        app: r.app,
                        name: r.name,
                        created_at: r.created_at,
                        last_used_at: r.last_used_at,
                    }
                })
                .collect();

            api_reply(Ok(TokenListResponse { app, tokens }))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_revoke_token(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path((app, name)): Path<(String, String)>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    match state.db.revoke_agent_token(&app, &name) {
        Ok(revoked) => {
            let _ = state.db.record_audit(
                &subject.identifier(),
                "admin",
                "token_revoke",
                &format!("{app}/{name}"),
                &format!("{revoked} token(s) revoked"),
            );
            api_reply(Ok(TokenRevokeResponse { app, name, revoked }))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}
