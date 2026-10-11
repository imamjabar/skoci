use chrono::Utc;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{SkociConfig, SkociEvidence};
use skoci_api::responses::{DeployResponse, DeployStage, StageStatus};
use skoci_core::config::ConfigLoader;
use skoci_node::archive::extract_tar_gz;
use skoci_node::builder::{BuildOptions, ImageBuilder};
use skoci_node::docker::{ContainerConfig, PortPublish};
use skoci_node::services::{ServiceManager, ServiceSpec};
use skoci_proxy::health::HealthChecker;
use skoci_proxy::router::Route;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::mpsc;

use crate::config::TlsMode;
use crate::db::{DeploymentRecord, RouteRecord};
use crate::events::JobEventBuffer;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Deploy,
    Rollback,
    Promote,
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::Deploy => "deploy",
            JobKind::Rollback => "rollback",
            JobKind::Promote => "promote",
        }
    }
}

pub struct DeployJobOptions {
    pub deployment_id: String,
    pub app: String,
    pub env: String,
    pub version: String,
    pub kind: JobKind,
    pub actor: String,
    pub commit: Option<String>,
    pub tar_gz_path: Option<PathBuf>,
    pub existing_image: Option<String>,
}

pub async fn run_deploy_pipeline(
    state: AppState,
    opts: DeployJobOptions,
    buffer: Arc<JobEventBuffer>,
) {
    let start_time = Instant::now();
    tracing::info!(
        deployment_id = %opts.deployment_id,
        app = %opts.app,
        env = %opts.env,
        kind = %opts.kind.as_str(),
        "Deployment started"
    );

    let res = execute_pipeline(&state, &opts, buffer.clone()).await;

    state.jobs.finish_job(&opts.app, &opts.env, &opts.deployment_id);

    // Clean up temporary files on completion (success or failure)
    let build_dir = state.config.data_dir.join("builds").join(&opts.deployment_id);
    let _ = fs::remove_dir_all(&build_dir).await;
    if let Some(tar_path) = &opts.tar_gz_path {
        let _ = fs::remove_file(tar_path).await;
    }

    match res {
        Ok((_deploy_response, _evidence)) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            tracing::info!(
                deployment_id = %opts.deployment_id,
                duration_ms = duration_ms,
                "Deployment succeeded"
            );

            // Record audit
            let _ = state.db.record_audit(
                &opts.actor,
                actor_type(&opts.actor),
                opts.kind.as_str(),
                &format!("{}/{}", opts.app, opts.env),
                &format!("Version {} successfully deployed", opts.version),
            );
        }
        Err(err) => {
            tracing::error!(
                deployment_id = %opts.deployment_id,
                error = %err.message,
                "Deployment failed"
            );

            buffer.emit(
                DeployStage::Done,
                StageStatus::Failed,
                format!("Deployment failed: {}", err.message),
                100,
                None,
                Some(err.clone()),
            );

            let mut rec = state
                .db
                .get_deployment(&opts.deployment_id)
                .ok()
                .flatten()
                .unwrap_or_else(|| DeploymentRecord {
                    id: opts.deployment_id.clone(),
                    app: opts.app.clone(),
                    env: opts.env.clone(),
                    version: opts.version.clone(),
                    kind: opts.kind.as_str().to_string(),
                    status: "failed".to_string(),
                    commit_hash: opts.commit.clone().unwrap_or_default(),
                    image: opts.existing_image.clone().unwrap_or_default(),
                    container: "".to_string(),
                    host_port: None,
                    triggered_by: opts.actor.clone(),
                    created_at: Utc::now(),
                    finished_at: Some(Utc::now()),
                    error_json: None,
                    evidence_json: None,
                });

            rec.status = "failed".to_string();
            rec.finished_at = Some(Utc::now());
            rec.error_json = serde_json::to_string(&err).ok();
            let _ = state.db.update_deployment(&rec);

            let _ = state.db.record_audit(
                &opts.actor,
                actor_type(&opts.actor),
                opts.kind.as_str(),
                &format!("{}/{}", opts.app, opts.env),
                &format!("Failed: {}", err.message),
            );
        }
    }
}

async fn execute_pipeline(
    state: &AppState,
    opts: &DeployJobOptions,
    buffer: Arc<JobEventBuffer>,
) -> SkociResult<(DeployResponse, SkociEvidence)> {
    let start_time = Instant::now();
    let build_dir = state.config.data_dir.join("builds").join(&opts.deployment_id);

    // Initial deployment record
    let mut rec = DeploymentRecord {
        id: opts.deployment_id.clone(),
        app: opts.app.clone(),
        env: opts.env.clone(),
        version: opts.version.clone(),
        kind: opts.kind.as_str().to_string(),
        status: "running".to_string(),
        commit_hash: opts.commit.clone().unwrap_or_else(|| "none".to_string()),
        image: opts
            .existing_image
            .clone()
            .unwrap_or_else(|| format!("skoci/{}:{}", opts.app, opts.version)),
        container: "".to_string(),
        host_port: None,
        triggered_by: opts.actor.clone(),
        created_at: Utc::now(),
        finished_at: None,
        error_json: None,
        evidence_json: None,
    };
    state.db.insert_deployment(&rec)?;

    let (cfg, image_tag) = match opts.kind {
        JobKind::Deploy => {
            // Stage 1: Extract (5 -> 10)
            buffer.emit(
                DeployStage::Extract,
                StageStatus::Running,
                "Extracting source code archive...",
                5,
                None,
                None,
            );

            let tar_path = opts.tar_gz_path.as_ref().ok_or_else(|| {
                SkociError::bad_request("Upload archive file not found", "Try deploying again")
            })?;

            let max_unpacked = state.config.max_upload_mb * 1024 * 1024 * 4;
            extract_tar_gz(tar_path, &build_dir, max_unpacked)?;

            let config_path = build_dir.join("skoci.yaml");
            if !config_path.exists() {
                return Err(SkociError::config_invalid(
                    "skoci.yaml file not found in project root",
                    "Run 'skoci init' to create skoci.yaml",
                ));
            }

            let config_content = fs::read_to_string(&config_path).await.map_err(|e| {
                SkociError::config_invalid(
                    format!("Failed to read skoci.yaml: {e}"),
                    "Check skoci.yaml syntax",
                )
            })?;

            if let Ok(meta) = std::fs::symlink_metadata(&config_path) {
                if meta.file_type().is_symlink() || !meta.is_file() {
                    return Err(SkociError::config_invalid(
                        "skoci.yaml must be a regular file, not a symlink",
                        "Replace the symlink with a regular skoci.yaml file",
                    ));
                }
            }

            let cfg = ConfigLoader::parse_config(&config_content)?;
            if cfg.app != opts.app {
                return Err(SkociError::config_invalid(
                    format!(
                        "App name in skoci.yaml ('{}') does not match parameter ('{}')",
                        cfg.app, opts.app
                    ),
                    "Adjust 'app' in skoci.yaml or deploy target",
                ));
            }

            let requested_hosts = compute_hosts(&state.config.base_domain, &cfg, &opts.env);
            for h in &requested_hosts {
                if let Some(api_d) = &state.config.api_domain {
                    if h.eq_ignore_ascii_case(api_d) {
                        return Err(SkociError::forbidden(
                            format!("Hostname '{h}' is reserved for the Skoci control API"),
                            "Remove this domain from skoci.yaml domains/preview_domains",
                        ));
                    }
                }
                if let Some(base_d) = &state.config.base_domain {
                    let base_suffix = format!(".{base_d}");
                    let auto_prod = format!("{}.{base_d}", cfg.app).to_lowercase();
                    let auto_prev = format!("{}-preview.{base_d}", cfg.app).to_lowercase();
                    if h.ends_with(&base_suffix) && h != &auto_prod && h != &auto_prev {
                        return Err(SkociError::forbidden(
                            format!("Hostname '{h}' uses the base domain but does not match this app's auto-hostnames"),
                            format!("Use auto-hostnames '{auto_prod}' or '{auto_prev}', or configure a custom domain"),
                        ));
                    }
                }
                let routes = state.db.list_routes()?;
                for r in routes {
                    if r.app != opts.app && r.hosts.iter().any(|existing| existing.eq_ignore_ascii_case(h)) {
                        return Err(SkociError::conflict(
                            format!("Hostname '{h}' is already owned by app '{}'", r.app),
                            "Choose a different domain name in skoci.yaml",
                        ));
                    }
                }
            }

            check_production_domain_policy(state, &opts.app, &opts.env, &cfg)?;
            check_production_dns_records(state, &opts.env, &cfg).await?;

            // Validate required secrets
            let existing_secrets = state.db.list_secret_keys_for_app(&opts.app)?;
            let secret_set: HashSet<String> = existing_secrets.into_iter().collect();
            for req_sec in &cfg.env.secrets {
                if !secret_set.contains(req_sec) {
                    return Err(SkociError::secret_missing(req_sec));
                }
            }

            // Save app config to DB
            state.db.save_app_config(&opts.app, &cfg)?;
            state.db.ensure_app(&opts.app)?;

            buffer.emit(
                DeployStage::Extract,
                StageStatus::Ok,
                "skoci.yaml extraction and validation succeeded",
                10,
                None,
                None,
            );

            // Stage 2: Build (10 -> 60)
            buffer.emit(
                DeployStage::Build,
                StageStatus::Running,
                format!("Building Docker image skoci/{}:{}...", opts.app, opts.version),
                10,
                None,
                None,
            );

            // Canonicalize and validate paths
            let canonical_build_dir = build_dir
                .canonicalize()
                .map_err(|e| SkociError::internal(format!("Canonicalize build dir failed: {e}"), "Try again"))?;

            let raw_context = build_dir.join(&cfg.build.context);
            let canonical_context = raw_context.canonicalize().map_err(|e| {
                SkociError::bad_request(
                    format!("Context path '{}' is invalid: {e}", cfg.build.context),
                    "Check build.context in skoci.yaml",
                )
            })?;

            if !canonical_context.starts_with(&canonical_build_dir) {
                return Err(SkociError::bad_request(
                    "build.context escapes build directory",
                    "Ensure build.context path is inside repository",
                ));
            }

            let raw_dockerfile = canonical_build_dir.join(&cfg.build.dockerfile);
            let canonical_dockerfile = raw_dockerfile.canonicalize().map_err(|e| {
                SkociError::bad_request(
                    format!("Dockerfile '{}' not found: {e}", cfg.build.dockerfile),
                    "Check build.dockerfile in skoci.yaml",
                )
            })?;

            if !canonical_dockerfile.starts_with(&canonical_build_dir) {
                return Err(SkociError::bad_request(
                    "build.dockerfile escapes build directory",
                    "Ensure Dockerfile path is inside repository",
                ));
            }

            let image_tag = format!("skoci/{}:{}", opts.app, opts.version);
            let build_opts = BuildOptions {
                tag: image_tag.clone(),
                dockerfile: canonical_dockerfile,
                context: canonical_context,
                labels: vec![
                    ("skoci.app".to_string(), opts.app.clone()),
                    ("skoci.env".to_string(), opts.env.clone()),
                    ("skoci.version".to_string(), opts.version.clone()),
                ],
            };

            let (log_tx, mut log_rx) = mpsc::unbounded_channel::<String>();
            let log_forwarder = {
                let buffer_clone = buffer.clone();
                tokio::spawn(async move {
                    while let Some(line) = log_rx.recv().await {
                        buffer_clone.emit(
                            DeployStage::Build,
                            StageStatus::Log,
                            line,
                            35,
                            None,
                            None,
                        );
                    }
                })
            };

            ImageBuilder::build(&build_opts, log_tx).await?;
            let _ = log_forwarder.await;

            buffer.emit(
                DeployStage::Build,
                StageStatus::Ok,
                format!("Image {image_tag} successfully built"),
                60,
                None,
                None,
            );

            (cfg, image_tag)
        }
        JobKind::Rollback | JobKind::Promote => {
            // Reusing existing image; Extract and Build are skipped
            buffer.emit(
                DeployStage::Extract,
                StageStatus::Skipped,
                "Extraction skipped (using cached version image)",
                10,
                None,
                None,
            );
            buffer.emit(
                DeployStage::Build,
                StageStatus::Skipped,
                "Build skipped (using cached version image)",
                60,
                None,
                None,
            );

            let cfg = state
                .db
                .get_app_config(&opts.app)?
                .ok_or_else(|| {
                    SkociError::bad_request(
                        format!("App config for '{}' not found in database", opts.app),
                        "Deploy app first",
                    )
                })?;

            let image_tag = opts
                .existing_image
                .clone()
                .unwrap_or_else(|| format!("skoci/{}:{}", opts.app, opts.version));

            (cfg, image_tag)
        }
    };

    check_production_domain_policy(state, &opts.app, &opts.env, &cfg)?;
    check_production_dns_records(state, &opts.env, &cfg).await?;

    // Stage 3: Services (60 -> 65)
    if cfg.services.is_empty() {
        buffer.emit(
            DeployStage::Services,
            StageStatus::Skipped,
            "No additional database/services configured",
            65,
            None,
            None,
        );
    } else {
        buffer.emit(
            DeployStage::Services,
            StageStatus::Running,
            format!("Setting up {} database service(s)...", cfg.services.len()),
            60,
            None,
            None,
        );

        let network_name = ServiceManager::network_name(&opts.app);
        state.docker.ensure_network(&network_name).await?;

        for (svc_name, svc_cfg) in &cfg.services {
            let pw_secret_key = ServiceManager::password_secret_key(svc_name);
            let password = match state.db.get_secret(&opts.app, &pw_secret_key)? {
                Some(sec) => state.secret_cipher.decrypt(&sec.ciphertext_hex, &sec.nonce_hex)?,
                None => {
                    let gen_pw = ServiceManager::generate_password();
                    let (ct, nonce) = state.secret_cipher.encrypt(&gen_pw)?;
                    state.db.set_secret(&opts.app, &pw_secret_key, &ct, &nonce)?;
                    gen_pw
                }
            };

            let spec = ServiceSpec {
                app: &opts.app,
                service: svc_name,
                cfg: svc_cfg,
                password: &password,
            };

            state.service_manager.ensure(&spec).await?;
        }

        buffer.emit(
            DeployStage::Services,
            StageStatus::Ok,
            "All database services are active and ready",
            65,
            None,
            None,
        );
    }

    // Stage 4: Start (65 -> 72)
    buffer.emit(
        DeployStage::Start,
        StageStatus::Running,
        "Starting application container...",
        65,
        None,
        None,
    );

    let container_name = format!("skoci-{}-{}-{}", opts.app, opts.env, opts.version);
    let network_name = ServiceManager::network_name(&opts.app);
    state.docker.ensure_network(&network_name).await?;

    // Create env file
    let envfiles_dir = state.config.data_dir.join("envfiles");
    fs::create_dir_all(&envfiles_dir).await.map_err(|e| {
        SkociError::internal(format!("Failed to create envfiles directory: {e}"), "Try again")
    })?;
    let env_file_path = envfiles_dir.join(format!("{}.env", opts.deployment_id));

    let mut env_lines = Vec::new();

    // 1. Public env
    for (k, v) in &cfg.env.public {
        env_lines.push(format!("{k}={v}"));
    }

    // 2. Decrypted secrets
    let mut secret_values: Vec<String> = Vec::new();
    for sec in state.db.list_secrets_for_app(&opts.app)? {
        let val = state
            .secret_cipher
            .decrypt(&sec.ciphertext_hex, &sec.nonce_hex)
            .map_err(|_| {
                SkociError::secret_missing(&sec.key)
            })?;
        env_lines.push(format!("{}={val}", sec.key));
        secret_values.push(val);
    }

    // 3. Service URLs
    for (svc_name, svc_cfg) in &cfg.services {
        let env_key = ServiceManager::env_var_name(svc_name);
        let pw_secret_key = ServiceManager::password_secret_key(svc_name);
        if let Some(sec) = state.db.get_secret(&opts.app, &pw_secret_key)? {
            if let Ok(pw) = state.secret_cipher.decrypt(&sec.ciphertext_hex, &sec.nonce_hex) {
                let spec = ServiceSpec {
                    app: &opts.app,
                    service: svc_name,
                    cfg: svc_cfg,
                    password: &pw,
                };
                let url = ServiceManager::connection_url(&spec);
                // Do not overwrite user-defined secret
                if !env_lines.iter().any(|l| l.starts_with(&format!("{env_key}="))) {
                    env_lines.push(format!("{env_key}={url}"));
                }
            }
        }
    }

    // 4. PORT default
    if !env_lines.iter().any(|l| l.starts_with("PORT=")) {
        env_lines.push(format!("PORT={}", cfg.run.port));
    }

    let env_content = env_lines.join("\n");
    fs::write(&env_file_path, &env_content).await.map_err(|e| {
        SkociError::internal(format!("Failed to write env file: {e}"), "Try again")
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&env_file_path, perms);
    }

    // An explicit host port survives container/daemon restarts; an ephemeral `127.0.0.1::port`
    // mapping would be reassigned and leave the proxy pointing at a dead port.
    let reserved: HashSet<u16> = state
        .db
        .list_routes()?
        .into_iter()
        .map(|r| r.host_port)
        .collect();
    let fixed_host_port = allocate_host_port(&reserved)?;

    let container_cfg = ContainerConfig {
        name: container_name.clone(),
        image: image_tag.clone(),
        publish: vec![PortPublish {
            host_ip: "127.0.0.1".to_string(),
            host_port: Some(fixed_host_port),
            container_port: cfg.run.port,
        }],
        env_file: Some(env_file_path.clone()),
        labels: vec![
            ("skoci.app".to_string(), opts.app.clone()),
            ("skoci.env".to_string(), opts.env.clone()),
            ("skoci.version".to_string(), opts.version.clone()),
        ],
        memory_limit: cfg.run.memory.clone().or_else(|| Some("1g".to_string())),
        cpu_limit: cfg.run.cpus,
        network: Some(network_name.clone()),
        network_aliases: vec![],
        volumes: vec![],
        restart_policy: Some("unless-stopped".to_string()),
        command: vec![],
        pids_limit: Some(1024),
    };

    if let Some(live) = state.router.get_route(&format!("{}:{}", opts.app, opts.env)).await {
        if live.version == opts.version {
            return Err(SkociError::conflict(
                format!("Version {} is already live in {}", opts.version, opts.env),
                "Choose another version with 'skoci rollback --to <version>'",
            ));
        }
    }
    let _ = state.docker.remove_container(&container_name).await;

    let run_res = state.docker.run_container(&container_cfg).await;
    // Always delete env file right after docker run returns
    let _ = fs::remove_file(&env_file_path).await;

    run_res?;

    let host_port = state
        .docker
        .host_port(&container_name, cfg.run.port)
        .await
        .map_err(|e| {
            SkociError::docker_error(format!("Failed to read container port: {e}"))
        })?;

    rec.container = container_name.clone();
    rec.host_port = Some(host_port);
    let _ = state.db.update_deployment(&rec);

    buffer.emit(
        DeployStage::Start,
        StageStatus::Ok,
        format!("Container active at 127.0.0.1:{host_port}"),
        72,
        None,
        None,
    );

    // Stage 5: Health (72 -> 85)
    buffer.emit(
        DeployStage::Health,
        StageStatus::Running,
        format!(
            "Checking health at http://127.0.0.1:{host_port}{} (timeout {}s)...",
            cfg.run.health, cfg.run.health_timeout_secs
        ),
        72,
        None,
        None,
    );

    let target_addr: SocketAddr = format!("127.0.0.1:{host_port}")
        .parse()
        .map_err(|e| SkociError::internal(format!("Failed to parse target address: {e}"), "Try again"))?;

    let health_checker = HealthChecker::new(Duration::from_secs(5));
    let health_res = health_checker
        .wait_for_healthy(
            target_addr,
            &cfg.run.health,
            Duration::from_secs(cfg.run.health_timeout_secs as u64),
        )
        .await;

    if let Err(health_err) = health_res {
        // Fetch last 40 container log lines
        if let Ok(log_lines) = state.docker.logs(&container_name, 40).await {
            let urls: Vec<String> = env_lines
                .iter()
                .filter_map(|l| l.split_once('=').map(|(_, v)| v.to_string()))
                .filter(|v| v.contains("://") && v.contains('@'))
                .collect();
            let sensitive: Vec<String> = secret_values.iter().cloned().chain(urls).collect();
            for line in log_lines {
                buffer.emit(
                    DeployStage::Health,
                    StageStatus::Log,
                    skoci_node::secret::redact(&line, &sensitive),
                    80,
                    None,
                    None,
                );
            }
        }

        // Remove the failed new container; old version remains untouched
        let _ = state.docker.stop_container(&container_name, 5).await;
        let _ = state.docker.remove_container(&container_name).await;

        return Err(health_err);
    }

    buffer.emit(
        DeployStage::Health,
        StageStatus::Ok,
        "Health check passed",
        85,
        None,
        None,
    );

    // Stage 6: Cutover (85 -> 90)
    buffer.emit(
        DeployStage::Cutover,
        StageStatus::Running,
        "Switching proxy traffic to new version...",
        85,
        None,
        None,
    );

    let hosts = compute_hosts(&state.config.base_domain, &cfg, &opts.env);
    let route = Route {
        app: opts.app.clone(),
        env: opts.env.clone(),
        hosts: hosts.clone(),
        target: target_addr,
        version: opts.version.clone(),
    };

    let prev_route = state.router.upsert_route(route).await?;

    let route_rec = RouteRecord {
        app: opts.app.clone(),
        env: opts.env.clone(),
        container: container_name.clone(),
        host_port,
        version: opts.version.clone(),
        hosts: hosts.clone(),
        deployed_at: Utc::now(),
    };
    state.db.upsert_route(&route_rec)?;

    // Set default app if none set yet
    if state.db.get_setting("default_app")?.is_none() {
        state.db.set_setting("default_app", &opts.app)?;
        state.router.set_default_app(Some(opts.app.clone())).await;
    }

    buffer.emit(
        DeployStage::Cutover,
        StageStatus::Ok,
        "Proxy traffic successfully switched",
        90,
        None,
        None,
    );

    // Stage 7: Smoke (90 -> 95)
    buffer.emit(
        DeployStage::Smoke,
        StageStatus::Running,
        "Running smoke test (GET /)...",
        90,
        None,
        None,
    );

    let smoke_status = health_checker.probe(target_addr, "/").await.ok();
    let smoke_passed = smoke_status.map(is_smoke_status_passed).unwrap_or(false);

    match smoke_status {
        Some(status) if smoke_passed => buffer.emit(
            DeployStage::Smoke,
            StageStatus::Ok,
            format!("Smoke test passed (GET / → {status})"),
            95,
            None,
            None,
        ),
        other => buffer.emit(
            DeployStage::Smoke,
            StageStatus::Failed,
            format!(
                "Smoke test failed (GET / → {}). Version remains live because health check passed; check 'skoci logs'",
                other.map(|s| s.to_string()).unwrap_or_else(|| "no response".to_string())
            ),
            95,
            None,
            None,
        ),
    }

    // Stage 8: Cleanup (95 -> 99)
    buffer.emit(
        DeployStage::Cleanup,
        StageStatus::Running,
        "Cleaning up previous version containers...",
        95,
        None,
        None,
    );

    if let Some(prev) = &prev_route {
        let prev_c = format!("skoci-{}-{}-{}", prev.app, prev.env, prev.version);
        if prev_c != container_name {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = state.docker.stop_container(&prev_c, 10).await;
            let _ = state.docker.remove_container(&prev_c).await;
        }
    }

    // Prune images: keep newest 3 per app, never delete images used by live routes
    prune_old_images(state, &opts.app).await;

    buffer.emit(
        DeployStage::Cleanup,
        StageStatus::Ok,
        "Container cleanup completed",
        99,
        None,
        None,
    );

    // Stage 9: Done (100)
    let urls = compute_urls(state, &opts.app, &opts.env, &hosts);
    if urls.is_empty() {
        buffer.emit(
            DeployStage::Cleanup,
            StageStatus::Log,
            format!(
                "Container is running but has no public address: no domain is configured for '{}' and it is not the default app. \
Fix: point a domain at this server and add it under 'domains'/'preview_domains' in skoci.yaml, or run 'skoci default-app {}' to serve it on the raw server IP.",
                opts.app, opts.app
            ),
            99,
            None,
            None,
        );
    }
    let previous_version = prev_route.as_ref().map(|r| r.version.clone());

    let duration_ms = start_time.elapsed().as_millis() as u64;

    let evidence = SkociEvidence {
        deployment_id: opts.deployment_id.clone(),
        app: opts.app.clone(),
        env: opts.env.clone(),
        commit: opts.commit.clone().unwrap_or_else(|| "none".to_string()),
        timestamp: Utc::now(),
        urls: urls.clone(),
        image: image_tag,
        container: container_name,
        health_check_url: format!("http://127.0.0.1:{host_port}{}", cfg.run.health),
        health_check_passed: true,
        smoke_test_status: smoke_status,
        smoke_test_passed: smoke_passed,
        previous_version,
        live_version: opts.version.clone(),
        triggered_by: opts.actor.clone(),
        approved_by: None,
        duration_ms,
    };

    let status_str = if smoke_passed { "live" } else { "degraded" };

    let deploy_response = DeployResponse {
        deployment_id: opts.deployment_id.clone(),
        app: opts.app.clone(),
        env: opts.env.clone(),
        version: opts.version.clone(),
        urls,
        status: status_str.to_string(),
        evidence: evidence.clone(),
    };

    rec.status = status_str.to_string();
    rec.finished_at = Some(Utc::now());
    rec.evidence_json = serde_json::to_string(&evidence).ok();
    state.db.update_deployment(&rec)?;

    let done_msg = if smoke_passed {
        format!("Deployment {} live!", opts.version)
    } else {
        format!("Deployment {} live (degraded: smoke test failed)!", opts.version)
    };

    buffer.emit(
        DeployStage::Done,
        StageStatus::Ok,
        done_msg,
        100,
        Some(deploy_response.clone()),
        None,
    );

    Ok((deploy_response, evidence))
}

pub fn compute_hosts(base_domain: &Option<String>, cfg: &SkociConfig, env: &str) -> Vec<String> {
    let mut hosts = Vec::new();

    if env == "production" {
        for d in &cfg.domains {
            let trimmed = d.trim().to_lowercase();
            if !trimmed.is_empty() && !hosts.contains(&trimmed) {
                hosts.push(trimmed);
            }
        }
        if let Some(base) = base_domain {
            let auto = format!("{}.{base}", cfg.app).to_lowercase();
            if !hosts.contains(&auto) {
                hosts.push(auto);
            }
        }
    } else {
        for d in &cfg.preview_domains {
            let trimmed = d.trim().to_lowercase();
            if !trimmed.is_empty() && !hosts.contains(&trimmed) {
                hosts.push(trimmed);
            }
        }
        if let Some(base) = base_domain {
            let auto = format!("{}-preview.{base}", cfg.app).to_lowercase();
            if !hosts.contains(&auto) {
                hosts.push(auto);
            }
        }
    }

    hosts
}

pub fn compute_urls(state: &AppState, app: &str, env: &str, hosts: &[String]) -> Vec<String> {
    let proto = match state.config.tls {
        TlsMode::Acme => "https",
        TlsMode::Off => "http",
    };
    let mut urls: Vec<String> = hosts.iter().map(|h| format!("{proto}://{h}")).collect();
    if let Some(ip) = state.config.public_ip {
        if serves_raw_ip(state, app, env) {
            urls.push(match ip {
                std::net::IpAddr::V4(v4) => format!("http://{v4}"),
                std::net::IpAddr::V6(v6) => format!("http://[{v6}]"),
            });
        }
    }
    urls
}

/// Mirrors the proxy rule: a raw-IP request goes to the default app's production route, else its preview route.
fn serves_raw_ip(state: &AppState, app: &str, env: &str) -> bool {
    if state.db.get_setting("default_app").ok().flatten().as_deref() != Some(app) {
        return false;
    }
    env == skoci_api::models::ENV_PRODUCTION
        || state
            .db
            .get_route(app, skoci_api::models::ENV_PRODUCTION)
            .ok()
            .flatten()
            .is_none()
}

async fn prune_old_images(state: &AppState, app: &str) {
    let routes = match state.db.list_routes() {
        Ok(r) => r,
        Err(_) => return,
    };
    let live_versions: HashSet<String> = routes.into_iter().map(|r| r.version).collect();

    let deps = match state.db.list_deployments(Some(app), 50) {
        Ok(d) => d,
        Err(_) => return,
    };

    let mut versions_by_age = Vec::new();
    for d in deps {
        if !versions_by_age.contains(&d.version) {
            versions_by_age.push(d.version);
        }
    }

    // Keep top 3 newest
    if versions_by_age.len() > 3 {
        for old_ver in &versions_by_age[3..] {
            if !live_versions.contains(old_ver) {
                let img = format!("skoci/{app}:{old_ver}");
                let _ = state.docker.remove_image(&img).await;
            }
        }
    }
}

fn actor_type(actor: &str) -> &'static str {
    if actor.starts_with("agent:") {
        "agent"
    } else {
        "human"
    }
}

fn allocate_host_port(reserved: &HashSet<u16>) -> SkociResult<u16> {
    for _ in 0..50 {
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .and_then(|l| l.local_addr())
            .map_err(|e| SkociError::internal(format!("Failed to find free port: {e}"), "Try deploying again"))?
            .port();
        if !reserved.contains(&port) {
            return Ok(port);
        }
    }
    Err(SkociError::internal("Could not find free port for container", "Try deploying again"))
}

pub fn is_smoke_status_passed(status: u16) -> bool {
    (200..=399).contains(&status) || status == 401
}

pub fn check_production_domain_policy(
    state: &AppState,
    app: &str,
    env: &str,
    cfg: &SkociConfig,
) -> SkociResult<()> {
    if env != skoci_api::models::ENV_PRODUCTION {
        return Ok(());
    }

    let hosts = compute_hosts(&state.config.base_domain, cfg, env);
    if hosts.is_empty() {
        return Err(SkociError::production_requires_domain(app, &[]));
    }

    if !state.config.allow_magic_dns_production {
        let explicit_magic: Vec<String> = cfg
            .domains
            .iter()
            .filter(|d| skoci_api::models::is_magic_dns(d))
            .cloned()
            .collect();
        let all_magic = hosts.iter().all(|h| skoci_api::models::is_magic_dns(h));

        if !explicit_magic.is_empty() || all_magic {
            let mut rejected = explicit_magic;
            for h in &hosts {
                if skoci_api::models::is_magic_dns(h) && !rejected.contains(h) {
                    rejected.push(h.clone());
                }
            }
            return Err(SkociError::production_requires_domain(app, &rejected));
        }
    }

    Ok(())
}

pub async fn check_production_dns_records(
    state: &AppState,
    env: &str,
    cfg: &SkociConfig,
) -> SkociResult<()> {
    if env != skoci_api::models::ENV_PRODUCTION {
        return Ok(());
    }

    let public_ip = match state.config.public_ip {
        Some(ip) => ip,
        None => return Ok(()),
    };

    let hosts = compute_hosts(&state.config.base_domain, cfg, env);
    let expected = vec![public_ip.to_string()];

    for host in hosts {
        if skoci_api::models::is_magic_dns(&host) {
            continue;
        }

        let resolved = crate::api::domains::resolve_host_ips(&host).await;
        if !resolved.contains(&public_ip) {
            let resolved_strs: Vec<String> = resolved.into_iter().map(|ip| ip.to_string()).collect();
            return Err(SkociError::dns_mismatch(&host, &expected, &resolved_strs));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smoke_classification_table() {
        let cases = [
            (Some(200), true),
            (Some(301), true),
            (Some(401), true),
            (Some(403), false),
            (Some(404), false),
            (Some(500), false),
            (None, false),
        ];
        for (status, expected_pass) in cases {
            let passed = status.map(is_smoke_status_passed).unwrap_or(false);
            assert_eq!(
                passed, expected_pass,
                "Failed classification for status {:?}",
                status
            );
        }
    }
}
