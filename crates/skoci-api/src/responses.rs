use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResponse {
    pub detected_framework: String,
    pub created_files: Vec<String>,
    pub app_name: String,
    /// Files that already existed and were left untouched.
    pub skipped_files: Vec<String>,
}

/// Returned immediately when a deploy, rollback or promotion job is queued.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployAccepted {
    pub deployment_id: String,
    pub app: String,
    pub env: String,
    pub version: String,
    /// Relative URL of the Server-Sent Events stream, e.g. `/api/deployments/<id>/events`.
    pub events_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployStage {
    Queued,
    Extract,
    Build,
    Services,
    Start,
    Health,
    Cutover,
    Smoke,
    Cleanup,
    Done,
}

impl DeployStage {
    /// Ordered stages shown as progress steps (excludes `Queued` and `Done`).
    pub const STEPS: [DeployStage; 8] = [
        DeployStage::Extract,
        DeployStage::Build,
        DeployStage::Services,
        DeployStage::Start,
        DeployStage::Health,
        DeployStage::Cutover,
        DeployStage::Smoke,
        DeployStage::Cleanup,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            DeployStage::Queued => "Queued",
            DeployStage::Extract => "Extract code",
            DeployStage::Build => "Build image",
            DeployStage::Services => "Prepare services",
            DeployStage::Start => "Start container",
            DeployStage::Health => "Health check",
            DeployStage::Cutover => "Switch traffic",
            DeployStage::Smoke => "Smoke test",
            DeployStage::Cleanup => "Clean up",
            DeployStage::Done => "Done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Running,
    Ok,
    Skipped,
    Failed,
    /// One output line (build log, container log) belonging to the current stage.
    Log,
}

/// One Server-Sent Event of `GET /api/deployments/:id/events` (SSE `data:` is this JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployEvent {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub stage: DeployStage,
    pub status: StageStatus,
    pub message: String,
    /// Overall progress 0..=100.
    pub progress: u8,
    /// Present only on the final event (`stage == Done`) of a successful job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<DeployResponse>,
    /// Present only on the final event of a failed job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::SkociError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployResponse {
    pub deployment_id: String,
    pub app: String,
    pub env: String,
    pub version: String,
    pub urls: Vec<String>,
    /// `live` or `failed`.
    pub status: String,
    pub evidence: crate::models::SkociEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub id: String,
    pub app: String,
    pub env: String,
    pub version: String,
    /// `kind`: `deploy`, `rollback` or `promote`.
    pub kind: String,
    /// `running`, `live`, `failed`, `superseded`.
    pub status: String,
    pub triggered_by: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<crate::error::SkociError>,
    pub evidence: Option<crate::models::SkociEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvStatus {
    pub env: String,
    pub version: String,
    pub container: String,
    /// Docker state: `running`, `exited`, `restarting`, `missing`, ...
    pub container_status: String,
    /// Result of a live health probe made while answering this request.
    pub healthy: bool,
    pub urls: Vec<String>,
    pub deployed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub app: String,
    pub environments: Vec<EnvStatus>,
    /// service name -> docker state of its container.
    pub services: BTreeMap<String, String>,
    pub is_default_app: bool,
    pub running_deployment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSummary {
    pub name: String,
    pub environments: Vec<EnvStatus>,
    pub is_default_app: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsResponse {
    pub app: String,
    pub env: String,
    pub container: String,
    pub lines: Vec<String>,
    /// Lines that look like errors (error/exception/panic/fatal/traceback), newest last.
    pub error_summary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretListResponse {
    pub app: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteResponse {
    pub app: String,
    pub version: String,
    /// `pending_approval` (agent) or `started` (human).
    pub status: String,
    pub request_id: Option<String>,
    pub deployment: Option<DeployAccepted>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalInfo {
    pub id: String,
    pub app: String,
    /// `promote`, `db_restore` or `app_delete`.
    pub action: String,
    pub detail: String,
    pub reason: String,
    /// `pending`, `approved`, `rejected`, `failed`.
    pub status: String,
    pub requested_by: String,
    pub decided_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub request_id: String,
    pub approved: bool,
    pub action: String,
    /// Set when approving started a deployment job.
    pub deployment: Option<DeployAccepted>,
    pub detail: String,
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
    pub version: String,
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
pub struct DbBackupListResponse {
    pub app: String,
    pub backups: Vec<DbBackupResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRestoreResponse {
    pub app: String,
    pub service: String,
    pub backup_id: String,
    /// `restored` (human) or `pending_approval` (agent).
    pub status: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDeleteResponse {
    pub app: String,
    /// `deleted` (human) or `pending_approval` (agent).
    pub status: String,
    pub request_id: Option<String>,
    pub removed_containers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCreateResponse {
    /// Shown exactly once; only its SHA-256 hash is stored.
    pub token: String,
    pub id: String,
    pub app: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// First 12 hex chars of the token hash. Safe to display and log.
    pub id: String,
    pub app: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenListResponse {
    pub app: String,
    pub tokens: Vec<TokenInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRevokeResponse {
    pub app: String,
    pub name: String,
    pub revoked: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    /// `admin` or `agent:<app>/<token name>#<token id>`. Never a raw token.
    pub actor: String,
    pub actor_type: String,
    pub action: String,
    pub target: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultAppResponse {
    pub app: Option<String>,
}
