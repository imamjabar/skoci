use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{ENV_PREVIEW, ENV_PRODUCTION};
use skoci_api::requests::PromoteRequest;
use skoci_api::responses::{DeployAccepted, PromoteResponse};

use crate::auth::Subject;
use crate::db::ApprovalRecord;
use crate::deploy::{run_deploy_pipeline, DeployJobOptions, JobKind};
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

fn approval_policy_base(state: &AppState, subject: &Subject) -> Option<bool> {
    if subject.is_admin() {
        return Some(false);
    }
    if state.config.require_approval_always {
        return Some(true);
    }
    None
}

/// Approval protects live production traffic; an app without a production route has none.
pub fn promotion_requires_approval(
    state: &AppState,
    subject: &Subject,
    app: &str,
) -> SkociResult<bool> {
    if let Some(forced) = approval_policy_base(state, subject) {
        return Ok(forced);
    }
    Ok(state.db.get_route(app, ENV_PRODUCTION)?.is_some())
}

/// Rollback approval policy:
/// - Admin or preview environment: never requires approval.
/// - require_approval_always: always requires approval.
/// - Production rollback to the immediately previous version (last known-good): allowed without approval.
/// - Production rollback to any other historical version: requires human approval.
pub fn rollback_requires_approval(
    state: &AppState,
    subject: &Subject,
    app: &str,
    env: &str,
    target_version: &str,
) -> SkociResult<bool> {
    if subject.is_admin() || env != ENV_PRODUCTION {
        return Ok(false);
    }
    if let Some(forced) = approval_policy_base(state, subject) {
        return Ok(forced);
    }

    let current_live = state
        .db
        .get_route(app, ENV_PRODUCTION)?
        .map(|r| r.version);

    let last_known_good = state
        .db
        .get_latest_successful_deployment(app, ENV_PRODUCTION, current_live.as_deref())?
        .map(|d| d.version);

    let is_last_known_good = last_known_good.as_deref() == Some(target_version);
    Ok(!is_last_known_good)
}

pub async fn handle_promote(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<PromoteRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }
    if let Ok(Some(cfg)) = state.db.get_app_config(&body.app) {
        if let Err(e) = crate::deploy::check_production_domain_policy(&state, &body.app, "production", &cfg) {
            return api_reply::<()>(Err(e));
        }
    }


    // Resolve version
    let version = match body.version {
        Some(v) => v,
        None => {
            let preview_route = match state.db.get_route(&body.app, ENV_PREVIEW) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return api_reply::<()>(Err(SkociError::bad_request(
                        "No active preview version to promote",
                        "Deploy to preview first",
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
                format!("Docker image '{image_tag}' not found"),
                "Deploy this version before promoting",
            )));
        }
        Err(e) => return api_reply::<()>(Err(e)),
    }

    let needs_approval = match promotion_requires_approval(&state, &subject, &body.app) {
        Ok(v) => v,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    if needs_approval {
        // Agent generates approval
        let approval_id = format!("appr_{}", uuid::Uuid::new_v4().simple());
        let rec = ApprovalRecord {
            id: approval_id.clone(),
            app: body.app.clone(),
            action: "promote".to_string(),
            detail: format!("Promote version {version} to production"),
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
            &format!("Approval request {approval_id} created for version {version}"),
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

    tokio::spawn(run_deploy_pipeline(state.clone(), opts, buffer));

    let estimated_secs = state
        .db
        .get_estimated_deploy_duration(&body.app, "production")
        .ok()
        .flatten();

    let accepted = DeployAccepted {
        deployment_id: deployment_id.clone(),
        app: body.app.clone(),
        env: "production".to_string(),
        version: version.clone(),
        events_url: format!("/api/deployments/{deployment_id}/events"),
        estimated_secs,
    };

    api_reply(Ok(PromoteResponse {
        app: body.app,
        version,
        status: "started".to_string(),
        request_id: None,
        deployment: Some(accepted),
    }))
}
