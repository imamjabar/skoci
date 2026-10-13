use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

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
}
