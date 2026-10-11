use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::models::DatabaseEngine;
use skoci_api::requests::ApproveRequest;
use skoci_api::responses::{ApprovalInfo, ApproveResponse, DeployAccepted};

use crate::api::apps::delete_app_resources;
use crate::deploy::{run_deploy_pipeline, DeployJobOptions, JobKind};
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_list_approvals(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
) -> Response {
    let app_filter = if subject.is_admin() {
        None
    } else {
        match &subject {
            crate::auth::Subject::Agent { app, .. } => Some(app.as_str()),
            _ => None,
        }
    };

    match state.db.list_pending_approvals(app_filter) {
        Ok(records) => {
            let infos: Vec<ApprovalInfo> = records.iter().map(|r| r.to_info()).collect();
            api_reply(Ok(infos))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_approve(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<ApproveRequest>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    let approval = match state.db.get_approval(&body.request_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            return api_reply::<()>(Err(SkociError::not_found(
                format!("Persetujuan '{}' tidak ditemukan", body.request_id),
                "Periksa ID persetujuan",
            )));
        }
        Err(e) => return api_reply::<()>(Err(e)),
    };

    if approval.status != "pending" {
        return api_reply::<()>(Err(SkociError::bad_request(
            format!(
                "Persetujuan '{}' sudah diproses dengan status '{}'",
                body.request_id, approval.status
            ),
            "Pilih persetujuan yang masih berstatus pending",
        )));
    }

    // Execute the stored action
    match approval.action.as_str() {
        "promote" => {
            let payload: serde_json::Value = approval
                .payload_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let version = payload["version"].as_str().unwrap_or("").to_string();
            if version.is_empty() {
                let _ = state.db.update_approval_status(
                    &body.request_id,
                    "failed",
                    Some(&subject.identifier()),
                );
                return api_reply::<()>(Err(SkociError::bad_request(
                    "Payload persetujuan promosi tidak memiliki versi yang valid",
                    "Buat permintaan promosi baru",
                )));
            }

            let image_tag = format!("skoci/{}:{}", approval.app, version);
            let deployment_id = format!("dep_{}", uuid::Uuid::new_v4().simple());

            let buffer = match state
                .jobs
                .register_job(&approval.app, "production", &deployment_id)
            {
                Ok(b) => b,
                Err(e) => return api_reply::<()>(Err(e)),
            };

            let opts = DeployJobOptions {
                deployment_id: deployment_id.clone(),
                app: approval.app.clone(),
                env: "production".to_string(),
                version: version.clone(),
                kind: JobKind::Promote,
                actor: subject.identifier(),
                commit: None,
                tar_gz_path: None,
                existing_image: Some(image_tag),
            };

            tokio::spawn(run_deploy_pipeline(state.clone(), opts, buffer));

            let _ = state.db.update_approval_status(
                &body.request_id,
                "approved",
                Some(&subject.identifier()),
            );

            let _ = state.db.record_audit(
                &subject.identifier(),
                "admin",
                "approve_promote",
                &format!("{}/production", approval.app),
                &format!("Persetujuan {} disetujui", body.request_id),
            );

            let accepted = DeployAccepted {
                deployment_id: deployment_id.clone(),
                app: approval.app,
                env: "production".to_string(),
                version: version.clone(),
                events_url: format!("/api/deployments/{deployment_id}/events"),
            };

            api_reply(Ok(ApproveResponse {
                request_id: body.request_id,
                approved: true,
                action: "promote".to_string(),
                deployment: Some(accepted),
                detail: format!("Promosi versi {version} disetujui dan proses deployment dimulai"),
            }))
        }
        "db_restore" => {
            let payload: serde_json::Value = approval
                .payload_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let service = payload["service"].as_str().unwrap_or("db").to_string();
            let backup_id = payload["backup_id"].as_str().unwrap_or("").to_string();

            let app_cfg = state.db.get_app_config(&approval.app).ok().flatten();
            let engine = app_cfg
                .and_then(|c| c.services.get(&service).map(|s| s.engine))
                .unwrap_or(DatabaseEngine::Postgres);

            match state
                .backup_manager
                .restore_backup(&approval.app, &service, engine, &backup_id)
                .await
            {
                Ok(_) => {
                    let _ = state.db.update_approval_status(
                        &body.request_id,
                        "approved",
                        Some(&subject.identifier()),
                    );

                    let _ = state.db.record_audit(
                        &subject.identifier(),
                        "admin",
                        "approve_db_restore",
                        &format!("{}/{}", approval.app, service),
                        &format!("Restore backup {} disetujui dan berhasil", backup_id),
                    );

                    api_reply(Ok(ApproveResponse {
                        request_id: body.request_id,
                        approved: true,
                        action: "db_restore".to_string(),
                        deployment: None,
                        detail: format!("Restore backup '{backup_id}' berhasil dilakukan"),
                    }))
                }
                Err(e) => {
                    let _ = state.db.update_approval_status(
                        &body.request_id,
                        "failed",
                        Some(&subject.identifier()),
                    );

                    let _ = state.db.record_audit(
                        &subject.identifier(),
                        "admin",
                        "approve_db_restore_failed",
                        &format!("{}/{}", approval.app, service),
                        &format!("Restore gagal: {}", e.message),
                    );

                    api_reply::<()>(Err(e))
                }
            }
        }
        "app_delete" => {
            match delete_app_resources(&state, &approval.app).await {
                Ok(_) => {
                    let _ = state.db.update_approval_status(
                        &body.request_id,
                        "approved",
                        Some(&subject.identifier()),
                    );

                    let _ = state.db.record_audit(
                        &subject.identifier(),
                        "admin",
                        "approve_app_delete",
                        &approval.app,
                        &format!("Persetujuan hapus aplikasi {} dijalankan", approval.app),
                    );

                    api_reply(Ok(ApproveResponse {
                        request_id: body.request_id,
                        approved: true,
                        action: "app_delete".to_string(),
                        deployment: None,
                        detail: format!("Aplikasi '{}' berhasil dihapus", approval.app),
                    }))
                }
                Err(e) => {
                    let _ = state.db.update_approval_status(
                        &body.request_id,
                        "failed",
                        Some(&subject.identifier()),
                    );

                    api_reply::<()>(Err(e))
                }
            }
        }
        unknown => {
            let _ = state.db.update_approval_status(
                &body.request_id,
                "failed",
                Some(&subject.identifier()),
            );
            api_reply::<()>(Err(SkociError::bad_request(
                format!("Aksi persetujuan '{unknown}' tidak dikenali"),
                "Aksi tidak didukung",
            )))
        }
    }
}

pub async fn handle_reject(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<ApproveRequest>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    let approval = match state.db.get_approval(&body.request_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            return api_reply::<()>(Err(SkociError::not_found(
                format!("Persetujuan '{}' tidak ditemukan", body.request_id),
                "Periksa ID persetujuan",
            )));
        }
        Err(e) => return api_reply::<()>(Err(e)),
    };

    if approval.status != "pending" {
        return api_reply::<()>(Err(SkociError::bad_request(
            format!(
                "Persetujuan '{}' sudah diproses dengan status '{}'",
                body.request_id, approval.status
            ),
            "Pilih persetujuan yang masih berstatus pending",
        )));
    }

    if let Err(e) = state.db.update_approval_status(
        &body.request_id,
        "rejected",
        Some(&subject.identifier()),
    ) {
        return api_reply::<()>(Err(e));
    }

    let note_text = body
        .note
        .unwrap_or_else(|| "Ditolak oleh administrator".to_string());

    let _ = state.db.record_audit(
        &subject.identifier(),
        "admin",
        "reject_approval",
        &format!("{}/{}", approval.app, body.request_id),
        &format!("Alasan: {note_text}"),
    );

    api_reply(Ok(ApproveResponse {
        request_id: body.request_id,
        approved: false,
        action: approval.action,
        deployment: None,
        detail: format!("Permintaan ditolak: {note_text}"),
    }))
}
