use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `skoci.yaml` definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkociConfig {
    pub version: u32,
    pub app: String,
    #[serde(default = "default_server")]
    pub server: String,
    #[serde(default)]
    pub build: BuildConfig,
    pub run: RunConfig,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub env: EnvConfig,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceConfig>,
}

fn default_server() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_build_mode")]
    pub mode: BuildMode,
    #[serde(default = "default_dockerfile")]
    pub dockerfile: String,
    #[serde(default = "default_context")]
    pub context: String,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            mode: BuildMode::Server,
            dockerfile: default_dockerfile(),
            context: default_context(),
        }
    }
}

fn default_build_mode() -> BuildMode {
    BuildMode::Server
}

fn default_dockerfile() -> String {
    "Dockerfile".to_string()
}

fn default_context() -> String {
    ".".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    Server,
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    pub port: u16,
    #[serde(default = "default_health")]
    pub health: String,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub cpus: Option<f32>,
}

fn default_health() -> String {
    "/healthz".to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvConfig {
    #[serde(default)]
    pub public: Vec<String>,
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
    pub schedule: String,
    #[serde(default = "default_keep")]
    pub keep: u32,
    #[serde(default = "default_destination")]
    pub to: String,
}

fn default_keep() -> u32 {
    7
}

fn default_destination() -> String {
    "local".to_string()
}

/// `skoci.policy.yaml` definition
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkociPolicy {
    pub version: u32,
    #[serde(default)]
    pub preview: EnvPolicy,
    #[serde(default)]
    pub production: EnvPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvPolicy {
    #[serde(default = "default_action_policy")]
    pub deploy: ActionPermission,
    #[serde(default = "default_action_policy")]
    pub rollback: ActionPermission,
    #[serde(default = "default_approve_policy")]
    pub db_restore: ActionPermission,
    #[serde(default = "default_approve_policy")]
    pub delete: ActionPermission,
}

impl Default for EnvPolicy {
    fn default() -> Self {
        Self {
            deploy: ActionPermission::Auto,
            rollback: ActionPermission::Auto,
            db_restore: ActionPermission::Approve,
            delete: ActionPermission::Approve,
        }
    }
}

fn default_action_policy() -> ActionPermission {
    ActionPermission::Auto
}

fn default_approve_policy() -> ActionPermission {
    ActionPermission::Approve
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPermission {
    Auto,
    Approve,
    Deny,
}

/// `skoci.evidence.json` definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkociEvidence {
    pub app: String,
    pub commit: String,
    pub timestamp: DateTime<Utc>,
    pub url: String,
    pub build_mode: String,
    pub health_check_passed: bool,
    pub smoke_test_passed: bool,
    #[serde(default)]
    pub previous_version: Option<String>,
    pub live_version: String,
    pub triggered_by: String,
    #[serde(default)]
    pub approved_by: Option<String>,
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
