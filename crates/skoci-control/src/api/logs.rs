use axum::extract::{Path, Query, State};
use axum::response::Response;
use skoci_api::requests::LogsQuery;
use skoci_api::responses::LogsResponse;
use skoci_node::secret::redact;

use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_logs(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Response {
    if let Err(err) = subject.require_app(&app) {
        return api_reply::<()>(Err(err));
    }

    let env = match query.env {
        Some(e) => e,
        None => {
            // Default to most recently deployed environment
            let routes = state.db.list_routes_for_app(&app).unwrap_or_default();
            if let Some(r) = routes.first() {
                r.env.clone()
            } else {
                "production".to_string()
            }
        }
    };

    let container = if let Ok(Some(r)) = state.db.get_route(&app, &env) {
        r.container
    } else if let Ok(Some(d)) = state.db.get_latest_successful_deployment(&app, &env, None) {
        d.container
    } else {
        format!("skoci-{app}-{env}")
    };

    let raw_lines = match state.docker.logs(&container, query.tail).await {
        Ok(l) => l,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    // Decrypt secrets for redaction
    let mut secret_values = Vec::new();
    if let Ok(sec_records) = state.db.list_secrets_for_app(&app) {
        for s in sec_records {
            if let Ok(dec) = state.secret_cipher.decrypt(&s.ciphertext_hex, &s.nonce_hex) {
                if dec.len() >= 4 {
                    secret_values.push(dec);
                }
            }
        }
    }

    let mut redacted_lines = Vec::with_capacity(raw_lines.len());
    let mut error_summary = Vec::new();

    for line in raw_lines {
        let cleaned = redact(&line, &secret_values);

        let lower = cleaned.to_lowercase();
        if (lower.contains("error")
            || lower.contains("exception")
            || lower.contains("panic")
            || lower.contains("fatal")
            || lower.contains("traceback"))
            && error_summary.len() < 20 {
                error_summary.push(cleaned.clone());
            }

        redacted_lines.push(cleaned);
    }

    api_reply(Ok(LogsResponse {
        app,
        env,
        container,
        lines: redacted_lines,
        error_summary,
    }))
}
