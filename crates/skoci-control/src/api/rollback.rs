use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::requests::RollbackRequest;
use skoci_api::responses::DeployAccepted;
use crate::api::promote::rollback_requires_approval;
use crate::db::ApprovalRecord;
use crate::deploy::{run_deploy_pipeline, DeployJobOptions, JobKind};
use crate::http::{api_accepted, api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_rollback(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<RollbackRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }
    if body.env == skoci_api::models::ENV_PRODUCTION {
        if let Ok(Some(cfg)) = state.db.get_app_config(&body.app) {
            if let Err(e) = crate::deploy::check_production_domain_policy(&state, &body.app, &body.env, &cfg) {
                return api_reply::<()>(Err(e));
            }
        }
    }


    // Resolve target version
    let target_version = match body.target_version {
        Some(v) => v,
        None => {
            let current_live = state
                .db
                .get_route(&body.app, &body.env)
                .ok()
                .flatten()
                .map(|r| r.version);

            let prev_dep = match state.db.get_latest_successful_deployment(
                &body.app,
                &body.env,
                current_live.as_deref(),
            ) {
                Ok(Some(d)) => d,
                Ok(None) => {
                    return api_reply::<()>(Err(SkociError::bad_request(
                        "No previous deployment version found for rollback",
                        "No historical version available to restore",
                    )));
                }
                Err(e) => return api_reply::<()>(Err(e)),
            };
            prev_dep.version
        }
    };

    // Check if rollback requires approval
    let needs_approval = match rollback_requires_approval(
        &state,
        &subject,
        &body.app,
        &body.env,
        &target_version,
    ) {
        Ok(v) => v,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    if needs_approval {
        let approval_id = format!("appr_{}", uuid::Uuid::new_v4().simple());
        let rec = ApprovalRecord {
            id: approval_id.clone(),
            app: body.app.clone(),
            action: "rollback".to_string(),
            detail: format!("Rollback version {target_version} to {}", body.env),
            reason: format!("Rollback to version {target_version}"),
            status: "pending".to_string(),
            requested_by: subject.identifier(),
            decided_by: None,
            payload_json: serde_json::to_string(&serde_json::json!({
                "version": target_version,
                "env": body.env,
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
            "rollback_request",
            &format!("{}/{}", body.app, body.env),
            &format!("Approval request {approval_id} created for version {target_version}"),
        );

        return api_reply::<()>(Err(SkociError::approval_required(&approval_id, "rollback")));
    }

    let image_tag = format!("skoci/{}:{}", body.app, target_version);
    match state.docker.image_exists(&image_tag).await {
        Ok(true) => {}
        Ok(false) => {
            return api_reply::<()>(Err(SkociError::bad_request(
                format!("Docker image '{image_tag}' not found for rollback"),
                "This image version is not available on Docker host",
            )));
        }
        Err(e) => return api_reply::<()>(Err(e)),
    }

    let deployment_id = format!("dep_{}", uuid::Uuid::new_v4().simple());
    let buffer = match state
        .jobs
        .register_job(&body.app, &body.env, &deployment_id)
    {
        Ok(b) => b,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    let opts = DeployJobOptions {
        deployment_id: deployment_id.clone(),
        app: body.app.clone(),
        env: body.env.clone(),
        version: target_version.clone(),
        kind: JobKind::Rollback,
        actor: subject.identifier(),
        commit: None,
        tar_gz_path: None,
        existing_image: Some(image_tag),
    };

    tokio::spawn(run_deploy_pipeline(state.clone(), opts, buffer));

    let estimated_secs = state
        .db
        .get_estimated_deploy_duration(&body.app, &body.env)
        .ok()
        .flatten();

    let accepted = DeployAccepted {
        deployment_id: deployment_id.clone(),
        app: body.app,
        env: body.env,
        version: target_version,
        events_url: format!("/api/deployments/{deployment_id}/events"),
        estimated_secs,
    };

    api_accepted(accepted, "Rollback accepted and queued for processing")
}
