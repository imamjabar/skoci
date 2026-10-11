use axum::extract::{Path, State};
use axum::response::Response;
use skoci_api::error::SkociResult;
use skoci_api::responses::{AppDeleteResponse, AppSummary, EnvStatus};
use skoci_proxy::health::HealthChecker;
use std::net::SocketAddr;
use std::time::Duration;

use crate::db::ApprovalRecord;
use crate::deploy::compute_urls;
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_list_apps(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
) -> Response {
    let apps_res = state.db.list_apps();
    let apps = match apps_res {
        Ok(a) => a,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    let default_app = state.db.get_setting("default_app").ok().flatten();
    let health_checker = HealthChecker::new(Duration::from_secs(3));

    let mut summaries = Vec::new();
    for app in apps {
        if !subject.can_access_app(&app.name) {
            continue;
        }

        let routes = state.db.list_routes_for_app(&app.name).unwrap_or_default();
        let app_cfg = state.db.get_app_config(&app.name).ok().flatten();
        let health_path = app_cfg
            .as_ref()
            .map(|c| c.run.health.as_str())
            .unwrap_or("/healthz");

        let mut env_statuses = Vec::new();
        for r in routes {
            let inspect = state
                .docker
                .inspect(&r.container)
                .await
                .unwrap_or_else(|_| skoci_node::docker::ContainerState {
                    exists: false,
                    running: false,
                    status: "missing".to_string(),
                    image: None,
                    started_at: None,
                });

            let target: Result<SocketAddr, _> = format!("127.0.0.1:{}", r.host_port).parse();
            let healthy = if inspect.running {
                if let Ok(addr) = target {
                    health_checker
                        .probe(addr, health_path)
                        .await
                        .map(|s| (200..400).contains(&s))
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            let urls = compute_urls(&state, &r.app, &r.env, &r.hosts);

            env_statuses.push(EnvStatus {
                env: r.env,
                version: r.version,
                container: r.container,
                container_status: inspect.status,
                healthy,
                urls,
                deployed_at: r.deployed_at,
            });
        }

        let is_default = default_app.as_deref() == Some(&app.name);
        summaries.push(AppSummary {
            name: app.name,
            environments: env_statuses,
            is_default_app: is_default,
            updated_at: app.updated_at,
        });
    }

    api_reply(Ok(summaries))
}

pub async fn handle_delete_app(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
) -> Response {
    if let Err(err) = subject.require_app(&app) {
        return api_reply::<()>(Err(err));
    }

    if !subject.is_admin() {
        // Agent generates an approval request
        let approval_id = format!("appr_{}", uuid::Uuid::new_v4().simple());
        let rec = ApprovalRecord {
            id: approval_id.clone(),
            app: app.clone(),
            action: "app_delete".to_string(),
            detail: format!("Delete app {app}"),
            reason: "App deletion requested by agent".to_string(),
            status: "pending".to_string(),
            requested_by: subject.identifier(),
            decided_by: None,
            payload_json: serde_json::to_string(&serde_json::json!({ "app": app })).ok(),
            created_at: chrono::Utc::now(),
        };

        if let Err(e) = state.db.insert_approval(&rec) {
            return api_reply::<()>(Err(e));
        }

        let _ = state.db.record_audit(
            &subject.identifier(),
            "agent",
            "app_delete_request",
            &app,
            &format!("Approval request {approval_id} created"),
        );

        return api_reply(Ok(AppDeleteResponse {
            app,
            status: "pending_approval".to_string(),
            request_id: Some(approval_id),
            removed_containers: vec![],
        }));
    }

    // Admin executes deletion directly
    let removed_containers = match delete_app_resources(&state, &app).await {
        Ok(c) => c,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    let _ = state.db.record_audit(
        &subject.identifier(),
        "admin",
        "app_delete",
        &app,
        &format!("App {app} deleted"),
    );

    api_reply(Ok(AppDeleteResponse {
        app,
        status: "deleted".to_string(),
        request_id: None,
        removed_containers,
    }))
}

pub async fn delete_app_resources(state: &AppState, app: &str) -> SkociResult<Vec<String>> {
    if let Some(running) = state.jobs.get_running_deployment(app) {
        return Err(skoci_api::error::SkociError::conflict(
            format!("Deployment {running} for '{app}' is still running"),
            format!("Wait until finished ('skoci watch {running}'), then delete again"),
        ));
    }

    // 1. Remove routes from proxy router
    let routes = state.db.list_routes_for_app(app)?;
    for r in routes {
        state.router.remove_route(&format!("{app}:{}", r.env)).await;
    }

    // 2. Stop and remove containers
    let containers = state
        .docker
        .list_containers(&format!("skoci.app={app}"))
        .await
        .unwrap_or_default();

    for c in &containers {
        let _ = state.docker.stop_container(c, 5).await;
        let _ = state.docker.remove_container(c).await;
    }

    // 3. Remove images
    let deps = state.db.list_deployments(Some(app), 100)?;
    for d in deps {
        let img = format!("skoci/{app}:{}", d.version);
        let _ = state.docker.remove_image(&img).await;
    }

    // 4. Delete app and DB entries
    state.db.delete_app(app)?;
    if state.db.get_setting("default_app")?.as_deref() == Some(app) {
        state.db.delete_setting("default_app")?;
        state.router.set_default_app(None).await;
    }

    Ok(containers)
}
