use crate::config::ServerConfig;
use crate::db::Database;
use crate::events::JobRegistry;
use crate::ratelimit::AuthRateLimiter;
use crate::session::SessionStore;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretCipher;
use skoci_node::services::ServiceManager;
use skoci_api::responses::ControlTlsInfo;
use skoci_proxy::{ProxyRouter, ProxyStatus};
use std::sync::Arc;

/// Live state of the TLS control endpoint, written by `tls::ControlTls`, read by doctor and deploy URLs.
pub type ControlTlsStatus = Arc<parking_lot::RwLock<Option<ControlTlsInfo>>>;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub db: Database,
    pub sessions: SessionStore,
    pub rate_limiter: AuthRateLimiter,
    pub jobs: JobRegistry,
    pub docker: DockerClient,
    pub router: ProxyRouter,
    pub secret_cipher: Arc<SecretCipher>,
    pub service_manager: ServiceManager,
    pub backup_manager: Arc<BackupManager>,
    /// None only in tests that build the router without starting the proxy.
    pub proxy_status: Option<ProxyStatus>,
    pub upload_semaphore: Arc<tokio::sync::Semaphore>,
    /// `None` inside when the TLS control endpoint is disabled.
    pub control_tls: ControlTlsStatus,
}
