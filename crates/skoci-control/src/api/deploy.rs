use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::StreamExt;
use skoci_api::error::SkociError;
use skoci_api::models::ENV_PRODUCTION;
use skoci_api::requests::{DeployRequest, DeploymentsQuery};
use skoci_api::responses::{
    DeployAccepted, DeployEvent, DeployResponse, DeployStage, DeploymentInfo, StageStatus,
};
use skoci_core::config::is_valid_app_name;
use std::convert::Infallible;
use std::time::Duration;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

use crate::deploy::{run_deploy_pipeline, DeployJobOptions, JobKind};
use crate::http::{api_accepted, api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_deploy(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Query(query): Query<DeployRequest>,
    body: Body,
) -> Response {
    if let Err(err) = subject.require_app(&query.app) {
        return api_reply::<()>(Err(err));
    }

    if query.env != ENV_PRODUCTION && query.env != skoci_api::models::ENV_PREVIEW {
        return api_reply::<()>(Err(SkociError::bad_request(
            format!("Unknown environment '{}'", query.env),
            "Use --env preview or --env production",
        )));
    }

    if !subject.is_admin() && query.env == ENV_PRODUCTION {
        return api_reply::<()>(Err(SkociError::new(
            "approval_required",
            "Agent tokens cannot deploy directly to production",
            "Deploy to preview first, then run 'skoci promote \"reason\"' and have an admin approve it",
        )));
    }

    if !is_valid_app_name(&query.app) {
        let err = SkociError::config_invalid(
            format!("App name '{}' is invalid", query.app),
            "Use an app name with lowercase letters, numbers, and '-'",
        );
        return api_reply::<()>(Err(err));
    }

    let deployment_id = format!("dep_{}", uuid::Uuid::new_v4().simple());
    let version = format!("v{}", Utc::now().format("%Y%m%d%H%M%S"));

    let uploads_dir = state.config.data_dir.join("uploads");
    if let Err(e) = fs::create_dir_all(&uploads_dir).await {
        return api_reply::<()>(Err(SkociError::internal(
            format!("Failed to create uploads directory: {e}"),
            "Check data directory permissions",
        )));
    }

    let tar_gz_path = uploads_dir.join(format!("{deployment_id}.tar.gz"));

    // Stream upload body and enforce byte limit
    let max_bytes = state.config.max_upload_mb * 1024 * 1024;
    let mut stream = body.into_data_stream();
    let mut file = match File::create(&tar_gz_path).await {
        Ok(f) => f,
        Err(e) => {
            return api_reply::<()>(Err(SkociError::internal(
                format!("Failed to create upload file: {e}"),
                "Check data directory permissions",
            )));
        }
    };

    let mut bytes_count = 0u64;
    while let Some(chunk_res) = stream.next().await {
        let chunk = match chunk_res {
            Ok(c) => c,
            Err(e) => {
                let _ = fs::remove_file(&tar_gz_path).await;
                return api_reply::<()>(Err(SkociError::bad_request(
                    format!("Failed to read upload data: {e}"),
                    "Try deploying again",
                )));
            }
        };

        bytes_count += chunk.len() as u64;
        if bytes_count > max_bytes {
            let _ = fs::remove_file(&tar_gz_path).await;
            return api_reply::<()>(Err(SkociError::payload_too_large(
                state.config.max_upload_mb,
            )));
        }

        if let Err(e) = file.write_all(&chunk).await {
            let _ = fs::remove_file(&tar_gz_path).await;
            return api_reply::<()>(Err(SkociError::internal(
                format!("Failed to write upload file: {e}"),
                "Check server disk space",
            )));
        }
    }

    if let Err(e) = file.flush().await {
        let _ = fs::remove_file(&tar_gz_path).await;
        return api_reply::<()>(Err(SkociError::internal(
            format!("Failed to flush upload file: {e}"),
            "Check server disk space",
        )));
    }

    let buffer = match state
        .jobs
        .register_job(&query.app, &query.env, &deployment_id)
    {
        Ok(b) => b,
        Err(e) => {
            let _ = fs::remove_file(&tar_gz_path).await;
            return api_reply::<()>(Err(e));
        }
    };

    let opts = DeployJobOptions {
        deployment_id: deployment_id.clone(),
        app: query.app.clone(),
        env: query.env.clone(),
        version: version.clone(),
        kind: JobKind::Deploy,
        actor: subject.identifier(),
        commit: query.commit,
        tar_gz_path: Some(tar_gz_path),
        existing_image: None,
    };

    tokio::spawn(run_deploy_pipeline(state.clone(), opts, buffer));

    let estimated_secs = state
        .db
        .get_estimated_deploy_duration(&query.app, &query.env)
        .ok()
        .flatten();

    let accepted = DeployAccepted {
        deployment_id: deployment_id.clone(),
        app: query.app,
        env: query.env,
        version,
        events_url: format!("/api/deployments/{deployment_id}/events"),
        estimated_secs,
    };

    api_accepted(accepted, "Deployment accepted and queued for processing")
}

pub async fn handle_list_deployments(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Query(query): Query<DeploymentsQuery>,
) -> Response {
    let app_filter = if subject.is_admin() {
        query.app.clone()
    } else {
        match &subject {
            crate::auth::Subject::Agent { app, .. } => {
                if let Some(req_app) = &query.app {
                    if req_app != app {
                        return api_reply::<()>(Err(SkociError::scope_violation(app, req_app)));
                    }
                }
                Some(app.clone())
            }
            _ => None,
        }
    };

    let limit = query.limit.unwrap_or(20);
    match state.db.list_deployments(app_filter.as_deref(), limit) {
        Ok(records) => {
            let infos: Vec<DeploymentInfo> = records.iter().map(|r| r.to_info()).collect();
            api_reply(Ok(infos))
        }
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_get_deployment(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(id): Path<String>,
) -> Response {
    match state.db.get_deployment(&id) {
        Ok(Some(rec)) => {
            if let Err(err) = subject.require_app(&rec.app) {
                return api_reply::<()>(Err(err));
            }
            api_reply(Ok(rec.to_info()))
        }
        Ok(None) => api_reply::<()>(Err(SkociError::not_found(
            format!("Deployment '{id}' not found"),
            "Check deployment ID",
        ))),
        Err(e) => api_reply::<()>(Err(e)),
    }
}

pub async fn handle_get_deployment_events(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(id): Path<String>,
) -> Response {
    if let Some(buf) = state.jobs.get_buffer(&id) {
        if let Err(err) = subject.require_app(&buf.app) {
            return api_reply::<()>(Err(err));
        }

        // Subscribe before snapshotting so no event can fall between the two; duplicates are
        // filtered by sequence number.
        let rx = buf.subscribe();
        let snapshot = buf.snapshot();

        let sse_state = EventStreamState {
            buffered: snapshot.into_iter(),
            rx,
            last_seq: 0,
            done: false,
        };

        let stream = futures_util::stream::unfold(sse_state, |mut s| async move {
            if s.done {
                return None;
            }
            let ev = match s.buffered.next() {
                Some(ev) => ev,
                None => loop {
                    match s.rx.recv().await {
                        Ok(ev) if ev.seq <= s.last_seq => continue,
                        Ok(ev) => break ev,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                },
            };
            s.last_seq = ev.seq;
            s.done = ev.stage == DeployStage::Done;
            let event = Event::default()
                .event("deploy")
                .data(serde_json::to_string(&ev).unwrap_or_default());
            Some((Ok::<_, Infallible>(event), s))
        });

        return Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text(""))
            .into_response();
    }

    // In-memory buffer is gone, check database to synthesize one done event
    match state.db.get_deployment(&id) {
        Ok(Some(rec)) => {
            if let Err(err) = subject.require_app(&rec.app) {
                return api_reply::<()>(Err(err));
            }

            let evidence: Option<skoci_api::models::SkociEvidence> = rec
                .evidence_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());
            let error = rec
                .error_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            let result = if rec.status == "live" {
                evidence.map(|ev| DeployResponse {
                    deployment_id: rec.id.clone(),
                    app: rec.app.clone(),
                    env: rec.env.clone(),
                    version: rec.version.clone(),
                    urls: ev.urls.clone(),
                    unready_urls: ev.unready_urls.clone(),
                    status: rec.status.clone(),
                    evidence: ev,
                })
            } else {
                None
            };

            let status = if rec.status == "live" {
                StageStatus::Ok
            } else {
                StageStatus::Failed
            };

            let ev = DeployEvent {
                seq: 1,
                ts: rec.finished_at.unwrap_or(rec.created_at),
                stage: DeployStage::Done,
                status,
                message: format!("Deployment {} completed ({})", rec.id, rec.status),
                progress: 100,
                result,
                error,
            };

            let data_str = serde_json::to_string(&ev).unwrap_or_default();
            let event = Event::default().event("deploy").data(data_str);

            let stream = futures_util::stream::once(async move { Ok::<_, Infallible>(event) });
            Sse::new(stream).into_response()
        }
        Ok(None) => api_reply::<()>(Err(SkociError::not_found(
            format!("Deployment '{id}' not found"),
            "Check deployment ID",
        ))),
        Err(e) => api_reply::<()>(Err(e)),
    }
}

struct EventStreamState {
    buffered: std::vec::IntoIter<DeployEvent>,
    rx: tokio::sync::broadcast::Receiver<DeployEvent>,
    last_seq: u64,
    done: bool,
}
