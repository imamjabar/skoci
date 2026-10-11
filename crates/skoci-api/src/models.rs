use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ENV_PREVIEW: &str = "preview";
pub const ENV_PRODUCTION: &str = "production";

/// Max length of an app name. Hostnames like `<app>-preview.<base>` need a DNS label <= 63 chars.
pub const MAX_APP_NAME_LEN: usize = 40;

/// Default TCP port of the TLS control endpoint (`https://<public-ip>:8443`).
pub const CONTROL_TLS_PORT: u16 = 8443;

/// Directory holding every file Skoci generates inside a user project.
pub const PROJECT_DIR: &str = ".skoci";
/// Project config, relative to the project root.
pub const PROJECT_CONFIG_PATH: &str = ".skoci/skoci.yaml";
/// Pre-0.4.1 project config location, still read so existing projects keep deploying.
pub const LEGACY_CONFIG_PATH: &str = "skoci.yaml";
/// Generated container recipe, relative to the project root.
pub const PROJECT_DOCKERFILE_PATH: &str = ".skoci/Dockerfile";
/// Upload filter for `skoci deploy`; separate from `.dockerignore`, which belongs to the user.
pub const PROJECT_IGNORE_PATH: &str = ".skoci/ignore";
/// Per-project server credentials, mode 0600, never uploaded.
pub const PROJECT_ENV_PATH: &str = ".skoci/env";
/// Pre-0.4.1 credential file, still read.
pub const LEGACY_ENV_PATH: &str = ".env.skoci";

/// Paths that an ignore rule must never remove from a deployment archive.
pub fn is_protected_upload_path(rel_path: &str, dockerfile: &str) -> bool {
    let p = rel_path.trim_start_matches("./");
    p == PROJECT_CONFIG_PATH
        || p == LEGACY_CONFIG_PATH
        || p == PROJECT_DIR
        || p == dockerfile.trim_start_matches("./")
}

/// Paths that must never enter a deployment archive, even when an ignore rule allows them.
pub fn is_forbidden_upload_path(rel_path: &str) -> bool {
    let p = rel_path.trim_start_matches("./");
    p == PROJECT_ENV_PATH || p == LEGACY_ENV_PATH
}

/// Third-party wildcard DNS zones that map `<ip>.<zone>` to an IP. Whoever controls the zone controls the name.
pub const MAGIC_DNS_ZONES: &[&str] = &[
    "sslip.io",
    "nip.io",
    "xip.io",
    "traefik.me",
    "localtest.me",
    "lvh.me",
    "nip.direct",
];

/// True when `host` is, or sits under, a known magic-DNS zone.
pub fn is_magic_dns(host: &str) -> bool {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    MAGIC_DNS_ZONES
        .iter()
        .any(|z| h == *z || (h.len() > z.len() && h.ends_with(z) && h.as_bytes()[h.len() - z.len() - 1] == b'.'))
}

/// Pin format: `sha256:<64 lowercase hex>` of a DER SubjectPublicKeyInfo.
pub fn spki_pin_from_der(spki_der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(spki_der)))
}

/// Normalizes user input (`sha256:<hex>` or bare hex, any case) and rejects anything else.
pub fn normalize_spki_pin(input: &str) -> Option<String> {
    let s = input.trim();
    let hex_part = s.strip_prefix("sha256:").or_else(|| s.strip_prefix("SHA256:")).unwrap_or(s);
    (hex_part.len() == 64 && hex_part.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("sha256:{}", hex_part.to_ascii_lowercase()))
}

/// `skoci.yaml` definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkociConfig {
    pub version: u32,
    pub app: String,
    #[serde(default)]
    pub build: BuildConfig,
    pub run: RunConfig,
    /// Production hostnames.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Preview hostnames.
    #[serde(default)]
    pub preview_domains: Vec<String>,
    #[serde(default)]
    pub env: EnvConfig,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_dockerfile")]
    pub dockerfile: String,
    #[serde(default = "default_context")]
    pub context: String,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            dockerfile: default_dockerfile(),
            context: default_context(),
        }
    }
}

fn default_dockerfile() -> String {
    "Dockerfile".to_string()
}

fn default_context() -> String {
    ".".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    pub port: u16,
    #[serde(default = "default_health")]
    pub health: String,
    #[serde(default = "default_health_timeout")]
    pub health_timeout_secs: u32,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub cpus: Option<f32>,
}

fn default_health() -> String {
    "/healthz".to_string()
}

fn default_health_timeout() -> u32 {
    60
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvConfig {
    /// Non-secret variables injected into the container.
    #[serde(default)]
    pub public: BTreeMap<String, String>,
    /// Names of secrets (set via `skoci secret set`) that must exist before deploy.
    #[serde(default)]
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub engine: DatabaseEngine,
    pub version: String,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub backup: Option<BackupPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseEngine {
    Postgres,
    Mysql,
    Valkey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPolicy {
    /// 5-field cron expression, evaluated in UTC.
    pub schedule: String,
    #[serde(default = "default_keep")]
    pub keep: u32,
}

fn default_keep() -> u32 {
    7
}

/// Evidence recorded for every finished deployment. Every field is measured, never assumed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkociEvidence {
    pub deployment_id: String,
    pub app: String,
    pub env: String,
    pub commit: String,
    pub timestamp: DateTime<Utc>,
    pub urls: Vec<String>,
    #[serde(default)]
    pub unready_urls: Vec<String>,
    pub image: String,
    pub container: String,
    pub health_check_url: String,
    pub health_check_passed: bool,
    /// HTTP status of `GET /` sent through the proxy after cutover.
    pub smoke_test_status: Option<u16>,
    pub smoke_test_passed: bool,
    #[serde(default)]
    pub previous_version: Option<String>,
    pub live_version: String,
    pub triggered_by: String,
    #[serde(default)]
    pub approved_by: Option<String>,
    pub duration_ms: u64,
}

/// Generic JSON response wrapper for agent-friendly output
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::SkociError>,
    pub summary: String,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T, summary: impl Into<String>) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            summary: summary.into(),
        }
    }

    pub fn err(error: crate::error::SkociError) -> Self {
        let summary = format!("{}: {}", error.code, error.message);
        Self {
            success: false,
            data: None,
            error: Some(error),
            summary,
        }
    }
}
