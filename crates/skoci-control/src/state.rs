use crate::config::ServerConfig;
use crate::db::Database;
use crate::events::JobRegistry;
use crate::ratelimit::AuthRateLimiter;
use crate::session::SessionStore;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretCipher;
use skoci_node::services::ServiceManager;
use skoci_proxy::{ProxyRouter, ProxyStatus};
use std::sync::Arc;

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
}
