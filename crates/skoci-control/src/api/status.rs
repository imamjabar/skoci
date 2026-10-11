use axum::extract::{Path, State};
use axum::response::Response;
use skoci_api::responses::{EnvStatus, StatusResponse};
use skoci_proxy::health::HealthChecker;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use crate::deploy::compute_urls;
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_status(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
) -> Response {
    if let Err(err) = subject.require_app(&app) {
        return api_reply::<()>(Err(err));
    }

    let routes = match state.db.list_routes_for_app(&app) {
        Ok(r) => r,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    let app_cfg = state.db.get_app_config(&app).ok().flatten();
    let health_path = app_cfg
        .as_ref()
        .map(|c| c.run.health.as_str())
        .unwrap_or("/healthz");

    let health_checker = HealthChecker::new(Duration::from_secs(3));
    let mut environments = Vec::new();

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

        environments.push(EnvStatus {
            env: r.env,
            version: r.version,
            container: r.container,
            container_status: inspect.status,
            healthy,
            urls,
            deployed_at: r.deployed_at,
        });
    }

    let mut services = BTreeMap::new();
    if let Some(cfg) = &app_cfg {
        for svc_name in cfg.services.keys() {
            let svc_state = state
                .service_manager
                .state(&app, svc_name)
                .await
                .map(|s| s.status)
                .unwrap_or_else(|_| "missing".to_string());
            services.insert(svc_name.clone(), svc_state);
        }
    }

    let default_app = state.db.get_setting("default_app").ok().flatten();
    let is_default_app = default_app.as_deref() == Some(&app);
    let running_deployment = state.jobs.get_running_deployment(&app);

    api_reply(Ok(StatusResponse {
        app,
        environments,
        services,
        is_default_app,
        running_deployment,
    }))
}
