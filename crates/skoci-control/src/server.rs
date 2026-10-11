use anyhow::Context;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretCipher;
use skoci_node::services::ServiceManager;
use skoci_proxy::router::Route;
use skoci_proxy::server::{start_proxy, AcmeSettings, ProxyConfig};
use skoci_proxy::ProxyRouter;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::net::TcpListener;

use crate::api::build_router;
use crate::config::{ServerConfig, TlsMode};
use crate::db::Database;
use crate::events::JobRegistry;
use crate::ratelimit::AuthRateLimiter;
use crate::scheduler::start_backup_scheduler;
use crate::session::SessionStore;
use crate::state::AppState;

pub async fn run_server(cfg: ServerConfig) -> anyhow::Result<()> {
    // Master token must be >= 32 characters
    if cfg.master_token.trim().len() < 32 {
        anyhow::bail!(
            "SKOCI_MASTER_TOKEN is required and must be at least 32 characters long (provided: {} characters)",
            cfg.master_token.trim().len()
        );
    }

    let data_dir = cfg.data_dir.clone();
    fs::create_dir_all(&data_dir)
        .await
        .with_context(|| format!("Failed to create data_dir at {}", data_dir.display()))?;

    fs::create_dir_all(data_dir.join("uploads")).await?;
    fs::create_dir_all(data_dir.join("builds")).await?;
    fs::create_dir_all(data_dir.join("envfiles")).await?;
    fs::create_dir_all(data_dir.join("backups")).await?;
    let acme_dir = data_dir.join("acme");
    fs::create_dir_all(&acme_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&acme_dir, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700));
    }

    // Clean up stale files from previous runs
    for dir_name in ["uploads", "builds", "envfiles"] {
        let p = data_dir.join(dir_name);
        if let Ok(mut entries) = fs::read_dir(&p).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    let _ = fs::remove_dir_all(entry.path()).await;
                } else {
                    let _ = fs::remove_file(entry.path()).await;
                }
            }
        }
    }
    let db = Database::open(data_dir.join("skoci.db"))
        .context("Failed to initialize SQLite database")?;

    let key_path = data_dir.join("node.key");
    let secret_cipher = Arc::new(
        SecretCipher::load_or_create(&key_path)
            .context("Failed to load or create node.key SecretCipher")?,
    );

    let docker = DockerClient::new();
    let service_manager = ServiceManager::new(docker.clone());
    let backup_manager = Arc::new(BackupManager::new(&data_dir, docker.clone()));
    let router = ProxyRouter::new();

    // Restore live routes from DB into ProxyRouter
    if let Ok(routes) = db.list_routes() {
        for mut r in routes {
            let is_running = match docker.inspect(&r.container).await {
                Ok(state) if state.running => true,
                Ok(state) if state.exists => docker.start_container(&r.container).await.is_ok(),
                _ => false,
            };

            if is_running {
                // Containers published on an ephemeral port get a new one after a restart.
                if let Ok(Some(cfg)) = db.get_app_config(&r.app) {
                    if let Ok(port) = docker.host_port(&r.container, cfg.run.port).await {
                        if port != r.host_port {
                            r.host_port = port;
                            let _ = db.upsert_route(&r);
                        }
                    }
                }
                let target = SocketAddr::from(([127, 0, 0, 1], r.host_port));

                let route = Route {
                    app: r.app.clone(),
                    env: r.env.clone(),
                    hosts: r.hosts.clone(),
                    target,
                    version: r.version.clone(),
                };
                let _ = router.upsert_route(route).await;
                tracing::info!(
                    app = %r.app,
                    env = %r.env,
                    "Route successfully restored to ProxyRouter"
                );
            } else {
                tracing::warn!(
                    app = %r.app,
                    env = %r.env,
                    container = %r.container,
                    "Container is not running, route not added to proxy"
                );
            }
        }
    }

    // Restore default app
    if let Ok(Some(default_app)) = db.get_setting("default_app") {
        router.set_default_app(Some(default_app)).await;
    }

    // Set control host
    router.set_control(cfg.api_domain.clone(), cfg.bind).await;

    // Start proxy
    let acme_settings = AcmeSettings {
        email: cfg.acme_email.clone(),
        cache_dir: data_dir.join("acme"),
        staging: cfg.acme_staging,
    };

    let acme = match cfg.tls {
        TlsMode::Acme => Some(acme_settings),
        TlsMode::Off => None,
    };

    let proxy_cfg = ProxyConfig {
        http_bind: cfg.http_bind,
        https_bind: cfg.https_bind,
        acme,
    };

    let proxy_status = start_proxy(proxy_cfg, router.clone()).await;

    let sessions = SessionStore::new();
    let rate_limiter = AuthRateLimiter::new();
    let jobs = JobRegistry::new();

    let app_state = AppState {
        config: Arc::new(cfg.clone()),
        db,
        sessions,
        rate_limiter,
        jobs: jobs.clone(),
        docker,
        router,
        secret_cipher,
        service_manager,
        backup_manager,
        proxy_status: Some(proxy_status),
        upload_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
    };
    // Eviction task for JobRegistry
    let jobs_clone = jobs.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            jobs_clone.evict_expired();
        }
    });

    // Start background backup scheduler
    tokio::spawn(start_backup_scheduler(app_state.clone()));

    // Axum server
    let app = build_router(app_state);
    let listener = TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("Failed to bind control API on {}", cfg.bind))?;

    tracing::info!("Skoci control plane active at http://{}", cfg.bind);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
