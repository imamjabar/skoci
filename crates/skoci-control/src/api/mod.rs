pub mod approval;
pub mod apps;
pub mod audits;
pub mod dashboard;
pub mod db;
pub mod default_app;
pub mod domains;
pub mod deploy;
pub mod doctor;
pub mod health;
pub mod logs;
pub mod promote;
pub mod rollback;
pub mod secret;
pub mod session;
pub mod status;
pub mod tokens;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::HeaderName;
use axum::http::HeaderValue;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::Router;

use crate::state::AppState;

async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    res
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Dashboard
        .route("/", get(dashboard::handle_dashboard))
        // Health
        .route("/api/health", get(health::handle_health))
        // Session
        .route("/api/session", post(session::handle_create_session).delete(session::handle_delete_session))
        // Deploy & Deployments
        .route(
            "/api/deploy",
            post(deploy::handle_deploy).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/deployments", get(deploy::handle_list_deployments))
        .route("/api/deployments/:id", get(deploy::handle_get_deployment))
        .route(
            "/api/deployments/:id/events",
            get(deploy::handle_get_deployment_events),
        )
        // Apps
        .route("/api/apps", get(apps::handle_list_apps))
        .route("/api/apps/:app", delete(apps::handle_delete_app))
        // Domains
        .route("/api/domains/check", post(domains::handle_domain_check))
        .route("/api/domains/primary", post(domains::handle_set_primary))
        // Status & Logs
        .route("/api/status/:app", get(status::handle_status))
        .route("/api/logs/:app", get(logs::handle_logs))
        // Secrets
        .route("/api/secret", post(secret::handle_set_secret))
        .route("/api/secret/:app", get(secret::handle_list_secrets))
        // Promote & Rollback
        .route("/api/promote", post(promote::handle_promote))
        .route("/api/rollback", post(rollback::handle_rollback))
        // Approvals
        .route("/api/approvals", get(approval::handle_list_approvals))
        .route("/api/approve", post(approval::handle_approve))
        .route("/api/reject", post(approval::handle_reject))
        // Database
        .route("/api/db/backup", post(db::handle_db_backup))
        .route("/api/db/backups/:app", get(db::handle_list_db_backups))
        .route("/api/db/restore", post(db::handle_db_restore))
        // Default App
        .route(
            "/api/default-app",
            get(default_app::handle_get_default_app).put(default_app::handle_set_default_app),
        )
        // Tokens
        .route("/api/tokens", post(tokens::handle_create_token))
        .route("/api/tokens/:app", get(tokens::handle_list_tokens))
        .route("/api/tokens/:app/:name", delete(tokens::handle_revoke_token))
        // Audits & Doctor
        .route("/api/audits", get(audits::handle_list_audits))
        .route("/api/doctor", get(doctor::handle_doctor))
        .layer(from_fn(security_headers_middleware))
        .with_state(state)
}
