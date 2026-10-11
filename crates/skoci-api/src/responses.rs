use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResponse {
    pub detected_framework: String,
    pub created_files: Vec<String>,
    pub app_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployResponse {
    pub app: String,
    pub version: String,
    pub env: String,
    pub url: String,
    pub status: String,
    pub health_check: bool,
    pub smoke_test: bool,
    pub evidence: crate::models::SkociEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub app: String,
    pub server: String,
    pub live_version: Option<String>,
    pub container_status: String,
    pub domains: Vec<String>,
    pub services: BTreeMap<String, String>,
    pub last_deploy: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsResponse {
    pub app: String,
    pub lines: Vec<String>,
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretListResponse {
    pub app: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteResponse {
    pub request_id: String,
    pub app: String,
    pub target_env: String,
    pub status: String, // "pending_approval" or "promoted"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub request_id: String,
    pub approved: bool,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackResponse {
    pub app: String,
    pub from_version: String,
    pub to_version: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorResponse {
    pub healthy: bool,
    pub idle_ram_mb: f32,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbBackupResponse {
    pub backup_id: String,
    pub app: String,
    pub service: String,
    pub size_bytes: u64,
    pub location: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRestoreResponse {
    pub app: String,
    pub service: String,
    pub restored_backup_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCreateResponse {
    pub token: String,
    pub app: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenListResponse {
    pub app: String,
    pub tokens: Vec<TokenInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub token_prefix: String,
    pub name: String,
    pub created_at: String,
}
