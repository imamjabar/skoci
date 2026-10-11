use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::models::DatabaseEngine;
use skoci_api::requests::{DbBackupRequest, DbRestoreRequest};
use skoci_api::responses::{DbBackupListResponse, DbRestoreResponse};

use crate::db::ApprovalRecord;
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_db_backup(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<DbBackupRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }

    let app_cfg = state.db.get_app_config(&body.app).ok().flatten();
    let engine = app_cfg
        .and_then(|c| c.services.get(&body.service).map(|s| s.engine))
        .unwrap_or(DatabaseEngine::Postgres);

    match state
        .backup_manager
        .create_backup(&body.app, &body.service, engine)
        .await
    {
        Ok(resp) => {
            let _ = state.db.record_audit(
                &subject.identifier(),
                subject.actor_type(),
                "db_backup",
                &format!("{}/{}", body.app, body.service),
                &format!("Backup {} dibuat", resp.backup_id),
            );
            api_reply(Ok(resp))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_list_db_backups(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
) -> Response {
    if let Err(err) = subject.require_app(&app) {
        return api_reply::<()>(Err(err));
    }

    match state.backup_manager.list_backups(&app) {
        Ok(backups) => api_reply(Ok(DbBackupListResponse { app, backups })),
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_db_restore(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<DbRestoreRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }

    // Validate backup_id format: [a-z0-9_-]+
    let is_valid_id = !body.backup_id.is_empty()
        && body
            .backup_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !is_valid_id {
        return api_reply::<()>(Err(SkociError::bad_request(
            format!("ID backup '{}' tidak valid", body.backup_id),
            "ID backup hanya boleh mengandung huruf kecil, angka, '_' dan '-'",
        )));
    }

    if !subject.is_admin() {
        // Agent generates an approval request
        let approval_id = format!("appr_{}", uuid::Uuid::new_v4().simple());
        let rec = ApprovalRecord {
            id: approval_id.clone(),
            app: body.app.clone(),
            action: "db_restore".to_string(),
            detail: format!(
                "Restore backup {} untuk service {}",
                body.backup_id, body.service
            ),
            reason: "Permintaan restore database oleh agent".to_string(),
            status: "pending".to_string(),
            requested_by: subject.identifier(),
            decided_by: None,
            payload_json: serde_json::to_string(&serde_json::json!({
                "service": body.service,
                "backup_id": body.backup_id,
            }))
            .ok(),
            created_at: chrono::Utc::now(),
        };

        if let Err(e) = state.db.insert_approval(&rec) {
            return api_reply::<()>(Err(e));
        }

        let _ = state.db.record_audit(
            &subject.identifier(),
            "agent",
            "db_restore_request",
            &format!("{}/{}", body.app, body.service),
            &format!(
                "Persetujuan {} dibuat untuk restore backup {}",
                approval_id, body.backup_id
            ),
        );

        return api_reply(Ok(DbRestoreResponse {
            app: body.app,
            service: body.service,
            backup_id: body.backup_id,
            status: "pending_approval".to_string(),
            request_id: Some(approval_id),
        }));
    }

    // Admin executes restore immediately
    let app_cfg = state.db.get_app_config(&body.app).ok().flatten();
    let engine = app_cfg
        .and_then(|c| c.services.get(&body.service).map(|s| s.engine))
        .unwrap_or(DatabaseEngine::Postgres);

    match state
        .backup_manager
        .restore_backup(&body.app, &body.service, engine, &body.backup_id)
        .await
    {
        Ok(resp) => {
            let _ = state.db.record_audit(
                &subject.identifier(),
                "admin",
                "db_restore",
                &format!("{}/{}", body.app, body.service),
                &format!("Backup {} berhasil di-restore", body.backup_id),
            );
            api_reply(Ok(resp))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}
