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

impl AppState {
    pub fn base_domain(&self) -> Option<String> {
        self.db
            .get_setting("base_domain")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.config.base_domain.clone())
    }

    pub fn api_domain(&self) -> Option<String> {
        if let Some(base) = self
            .db
            .get_setting("base_domain")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
        {
            Some(format!("skoci.{base}"))
        } else {
            self.config
                .api_domain
                .clone()
                .or_else(|| self.config.base_domain.as_ref().map(|b| format!("skoci.{b}")))
        }
    }
}
