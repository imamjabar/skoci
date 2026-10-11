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
    /// Public IP of the server; the default app is reported as reachable at `http://<ip>`.
    pub public_ip: Option<IpAddr>,
}
