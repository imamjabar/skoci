use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

pub const DEFAULT_RETENTION_DEPLOYMENTS_PER_APP: u32 = 30;
pub const DEFAULT_RETENTION_DEPLOYMENTS_MAX_AGE_DAYS: u32 = 90;
pub const DEFAULT_RETENTION_APPROVALS_MAX_AGE_DAYS: u32 = 30;
pub const DEFAULT_RETENTION_AUDIT_LOGS_MAX_AGE_DAYS: u32 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Off,
    Acme,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub master_token: String,
    pub http_bind: SocketAddr,
    pub https_bind: SocketAddr,
    pub base_domain: Option<String>,
    pub api_domain: Option<String>,
    pub tls: TlsMode,
    pub acme_email: Option<String>,
    pub acme_staging: bool,
    pub max_upload_mb: u64,
    /// Public IP of the server: raw-IP routing to the default app, IP certificate, DNS checks.
    pub public_ip: Option<IpAddr>,
    /// TLS control endpoint, pinned by SPKI (`https://<public-ip>:8443`). `None` disables it.
    pub control_tls_bind: Option<SocketAddr>,
    /// Operator-only escape hatch: allow magic-DNS hostnames (sslip.io, ...) in production.
    pub allow_magic_dns_production: bool,
    pub require_approval_always: bool,
    pub retention_deployments_per_app: u32,
    pub retention_deployments_max_age_days: u32,
    pub retention_approvals_max_age_days: u32,
    pub retention_audit_logs_max_age_days: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".parse().unwrap(),
            data_dir: PathBuf::from("/var/lib/skoci"),
            master_token: String::new(),
            http_bind: "127.0.0.1:80".parse().unwrap(),
            https_bind: "127.0.0.1:443".parse().unwrap(),
            base_domain: None,
            api_domain: None,
            tls: TlsMode::Off,
            acme_email: None,
            acme_staging: false,
            max_upload_mb: 100,
            public_ip: None,
            control_tls_bind: None,
            allow_magic_dns_production: false,
            require_approval_always: false,
            retention_deployments_per_app: DEFAULT_RETENTION_DEPLOYMENTS_PER_APP,
            retention_deployments_max_age_days: DEFAULT_RETENTION_DEPLOYMENTS_MAX_AGE_DAYS,
            retention_approvals_max_age_days: DEFAULT_RETENTION_APPROVALS_MAX_AGE_DAYS,
            retention_audit_logs_max_age_days: DEFAULT_RETENTION_AUDIT_LOGS_MAX_AGE_DAYS,
        }
    }
}
