use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRequest {
    pub app: String,
    #[serde(default)]
    pub env: String, // "preview" or "production"
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackRequest {
    pub app: String,
    #[serde(default)]
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteRequest {
    pub app: String,
    pub target_env: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub request_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerAddRequest {
    pub name: String,
    pub host: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsRequest {
    pub app: String,
    #[serde(default = "default_tail")]
    pub tail: usize,
    #[serde(default)]
    pub follow: bool,
}

fn default_tail() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCreateRequest {
    pub app: String,
    pub name: String,
}
