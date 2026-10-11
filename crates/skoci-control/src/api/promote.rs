use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::models::ENV_PREVIEW;
use skoci_api::requests::PromoteRequest;
use skoci_api::responses::{DeployAccepted, PromoteResponse};

use crate::db::ApprovalRecord;
use crate::deploy::{run_deploy_pipeline, DeployJobOptions, JobKind};
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_promote(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<PromoteRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }

    // Resolve version
    let version = match body.version {
        Some(v) => v,
        None => {
            let preview_route = match state.db.get_route(&body.app, ENV_PREVIEW) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return api_reply::<()>(Err(SkociError::bad_request(
                        "Tidak ada versi preview yang sedang aktif untuk dipromosikan",
                        "Jalankan deploy preview terlebih dahulu",
                    )));
                }
                Err(e) => return api_reply::<()>(Err(e)),
            };
            preview_route.version
        }
    };

    let image_tag = format!("skoci/{}:{}", body.app, version);
    match state.docker.image_exists(&image_tag).await {
        Ok(true) => {}
        Ok(false) => {
            return api_reply::<()>(Err(SkociError::bad_request(
                format!("Docker image '{image_tag}' tidak ditemukan"),
                "Deploy versi ini terlebih dahulu sebelum dipromosikan",
            )));
        }
        Err(e) => return api_reply::<()>(Err(e)),
    }

    if !subject.is_admin() {
        // Agent generates approval
        let approval_id = format!("appr_{}", uuid::Uuid::new_v4().simple());
        let rec = ApprovalRecord {
            id: approval_id.clone(),
            app: body.app.clone(),
            action: "promote".to_string(),
            detail: format!("Promosikan versi {version} ke production"),
            reason: body.reason,
            status: "pending".to_string(),
            requested_by: subject.identifier(),
            decided_by: None,
            payload_json: serde_json::to_string(&serde_json::json!({
                "version": version,
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
            "promote_request",
            &format!("{}/production", body.app),
            &format!("Persetujuan {approval_id} dibuat untuk versi {version}"),
        );

        return api_reply(Ok(PromoteResponse {
            app: body.app,
            version,
            status: "pending_approval".to_string(),
            request_id: Some(approval_id),
            deployment: None,
        }));
    }

    // Admin starts promote deployment job immediately
    let deployment_id = format!("dep_{}", uuid::Uuid::new_v4().simple());
    let buffer = match state
        .jobs
        .register_job(&body.app, "production", &deployment_id)
    {
        Ok(b) => b,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    let opts = DeployJobOptions {
        deployment_id: deployment_id.clone(),
        app: body.app.clone(),
        env: "production".to_string(),
        version: version.clone(),
        kind: JobKind::Promote,
        actor: subject.identifier(),
        commit: None,
        tar_gz_path: None,
        existing_image: Some(image_tag),
    };

    tokio::spawn(run_deploy_pipeline(state, opts, buffer));

    let accepted = DeployAccepted {
        deployment_id: deployment_id.clone(),
        app: body.app.clone(),
        env: "production".to_string(),
        version: version.clone(),
        events_url: format!("/api/deployments/{deployment_id}/events"),
    };

    api_reply(Ok(PromoteResponse {
        app: body.app,
        version,
        status: "started".to_string(),
        request_id: None,
        deployment: Some(accepted),
    }))
}
