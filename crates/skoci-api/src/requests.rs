use serde::{Deserialize, Serialize};

/// Query string of `POST /api/deploy`. The body is the gzip-compressed tar of the build context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRequest {
    pub app: String,
    #[serde(default = "default_env")]
    pub env: String,
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackRequest {
    pub app: String,
    #[serde(default = "default_env")]
    pub env: String,
    /// Defaults to the newest successful version older than the live one.
    #[serde(default)]
    pub target_version: Option<String>,
}

/// Promote a preview version to production.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteRequest {
    pub app: String,
    pub reason: String,
    /// Defaults to the live preview version.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub request_id: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretSetRequest {
    pub app: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbBackupRequest {
    pub app: String,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRestoreRequest {
    pub app: String,
    pub service: String,
    pub backup_id: String,
}

/// Query string of `GET /api/logs/:app`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsQuery {
    /// Defaults to the most recently deployed environment.
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default = "default_tail")]
    pub tail: usize,
}

/// Query string of `GET /api/deployments`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploymentsQuery {
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCreateRequest {
    pub app: String,
    pub name: String,
}

/// Dashboard login. Only the master token is accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRequest {
    pub token: String,
}

/// Choose which app answers requests addressed to the raw server IP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultAppRequest {
    pub app: String,
}

fn default_env() -> String {
    crate::models::ENV_PREVIEW.to_string()
}

fn default_tail() -> usize {
    100
}
