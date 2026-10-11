use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ENV_PREVIEW: &str = "preview";
pub const ENV_PRODUCTION: &str = "production";

/// Max length of an app name. Hostnames like `<app>-preview.<base>` need a DNS label <= 63 chars.
pub const MAX_APP_NAME_LEN: usize = 40;

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
