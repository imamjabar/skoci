use crate::protocol::{McpContent, McpTool, McpToolCallResult};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{
    DbBackupRequest, DbRestoreRequest, DeployRequest, DeploymentsQuery, LogsQuery,
    PromoteRequest, RollbackRequest, SecretSetRequest,
};
use skoci_api::responses::{DeployResponse, DeployStage, StageStatus};
use skoci_client::SkociClient;
use std::path::{Path, PathBuf};

fn wrap_api_response<T: Serialize>(res: SkociResult<ApiResponse<T>>) -> McpToolCallResult {
    match res {
        Ok(api_resp) => {
            let is_error = !api_resp.success;
            let text = serde_json::to_string_pretty(&api_resp)
                .unwrap_or_else(|e| format!("Failed to format JSON: {e}"));
            McpToolCallResult {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text,
                }],
                is_error: if is_error { Some(true) } else { None },
            }
        }
        Err(err) => {
            let api_err = ApiResponse::<()>::err(err);
            let text = serde_json::to_string_pretty(&api_err)
                .unwrap_or_else(|e| format!("Failed to format JSON: {e}"));
            McpToolCallResult {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text,
                }],
                is_error: Some(true),
            }
        }
    }
}

fn resolve_mcp_app(args: &serde_json::Value, default_path: &Path) -> SkociResult<String> {
    skoci_cli::resolve_app_in(default_path, args.get("app").and_then(|v| v.as_str()))
}

pub fn list_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "skoci_init".to_string(),
            description: "Detect project framework and generate .skoci/skoci.yaml, .skoci/Dockerfile, and .skoci/ignore".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project directory (default: .)" },
                    "app": { "type": "string", "description": "Optional application name" }
                }
            }),
        },
        McpTool {
            name: "skoci_deploy".to_string(),
            description: "Deploy project to Skoci server (pack, upload, and stream events until complete)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Project directory (default: .)" },
                    "env": { "type": "string", "description": "Target environment (preview or production, default: preview)" }
                }
            }),
        },
        McpTool {
            name: "skoci_watch".to_string(),
            description: "Watch a running deployment until completion and return final status".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "deployment_id": { "type": "string", "description": "Deployment ID to watch" }
                },
                "required": ["deployment_id"]
            }),
        },
        McpTool {
            name: "skoci_status".to_string(),
            description: "Check container status, live health, and application URLs".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_logs".to_string(),
            description: "View recent application logs with error line summary".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "env": { "type": "string", "description": "Application environment (optional)" },
                    "tail": { "type": "number", "description": "Number of recent log lines (default: 100)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_deployments".to_string(),
            description: "List application deployment history".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Optional application name" },
                    "limit": { "type": "number", "description": "Maximum history entries (default: 20)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_secret_set".to_string(),
            description: "Set encrypted secret for application".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "key": { "type": "string", "description": "Secret key name (uppercase and underscores)" },
                    "value": { "type": "string", "description": "Secret value" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                },
                "required": ["key", "value"]
            }),
        },
        McpTool {
            name: "skoci_secret_list".to_string(),
            description: "List application secret key names (values remain encrypted and hidden)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_promote".to_string(),
            description: "Submit promotion from preview to production (requires admin approval when called by agent)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "reason": { "type": "string", "description": "Promotion reason" },
                    "version": { "type": "string", "description": "Version to promote (optional)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                },
                "required": ["reason"]
            }),
        },
        McpTool {
            name: "skoci_rollback".to_string(),
            description: "Roll back application to a previous healthy version".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "env": { "type": "string", "description": "Target environment (default: preview)" },
                    "target_version": { "type": "string", "description": "Optional target rollback version" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_approvals".to_string(),
            description: "List pending approval requests waiting for admin action".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "skoci_doctor".to_string(),
            description: "Check health of Skoci server, Docker daemon, RAM, and proxy status".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "skoci_db_backup".to_string(),
            description: "Create secure application database backup".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "service": { "type": "string", "description": "Database service name (default: db)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_db_list".to_string(),
            description: "List database backup history for application".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_db_restore".to_string(),
            description: "Submit database restore from backup file (requires admin approval when called by agent)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name (optional if .skoci/skoci.yaml or skoci.yaml exists)" },
                    "backup_id": { "type": "string", "description": "Database backup ID" },
                    "service": { "type": "string", "description": "Database service name (default: db)" },
                    "path": { "type": "string", "description": "Directory containing .skoci/skoci.yaml or skoci.yaml (default: .)" }
                },
                "required": ["backup_id"]
            }),
        },
    ]
}

pub async fn call_tool(
    client: &SkociClient,
    name: &str,
    args: serde_json::Value,
) -> McpToolCallResult {
    let raw_path = PathBuf::from(
        args.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("."),
    );
    let default_path = match validate_mcp_path(&raw_path) {
        Ok(p) => p,
        Err(e) => return wrap_api_response::<()>(Err(e)),
    };

    match name {
        "skoci_init" => {
            let app = args.get("app").and_then(|v| v.as_str());
            let res = skoci_cli::init_project(&default_path, app);
            wrap_api_response(res.map(|r| ApiResponse::ok(r, "Project successfully initialized")))
        }

        "skoci_deploy" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };

            let env = args
                .get("env")
                .and_then(|v| v.as_str())
                .unwrap_or("preview")
                .to_string();

            let mut git_cmd = tokio::process::Command::new("git");
            git_cmd
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(&default_path)
                .env_clear();
            if let Ok(p) = std::env::var("PATH") {
                git_cmd.env("PATH", p);
            }
            if let Ok(h) = std::env::var("HOME") {
                git_cmd.env("HOME", h);
            }
            let out = git_cmd.output().await;
            let commit = match &out {
                Ok(o) if o.status.success() => {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                }
                _ => Some("none".to_string()),
            };

            let packed = match skoci_client::pack_context(&default_path) {
                Ok(p) => p,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };

            let req = DeployRequest {
                app,
                env,
                commit,
            };

            let upload_res = client.deploy_upload(&packed.path, &req, |_sent, _total| {}).await;
            let accepted = match upload_res {
                Ok(resp) => {
                    if !resp.success {
                        return wrap_api_response(Ok(resp));
                    }
                    match resp.data {
                        Some(d) => d,
                        None => {
                            return wrap_api_response::<()>(Err(SkociError::internal(
                                "Empty response from server",
                                "Try deploying again",
                            )))
                        }
                    }
                }
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };

            let mut stream = client.follow_events(&accepted.deployment_id);
            let mut log_lines = Vec::new();
            let mut final_response: Option<DeployResponse> = None;
            let mut final_error: Option<SkociError> = None;

            while let Some(res) = stream.next().await {
                match res {
                    Ok(ev) => {
                        if ev.status == StageStatus::Log {
                            log_lines.push(ev.message);
                            if log_lines.len() > 50 {
                                log_lines.remove(0);
                            }
                        }
                        if ev.stage == DeployStage::Done {
                            if let Some(r) = ev.result {
                                final_response = Some(r);
                            }
                            if let Some(e) = ev.error {
                                final_error = Some(e);
                            }
                        }
                    }
                    Err(err) => {
                        final_error = Some(err);
                        break;
                    }
                }
            }

            if let Some(resp) = final_response {
                wrap_api_response(Ok(ApiResponse::ok(resp, "Deployment successful")))
            } else if let Some(mut err) = final_error {
                if !log_lines.is_empty() {
                    let tail_logs = log_lines.iter().rev().take(30).rev().cloned().collect::<Vec<_>>().join("\n");
                    err.message = format!("{}\n\nRecent build/container logs:\n{}", err.message, tail_logs);
                }
                wrap_api_response::<DeployResponse>(Err(err))
            } else {
                wrap_api_response::<DeployResponse>(Err(SkociError::internal(
                    "Event stream disconnected without final status data",
                    format!("Use skoci_watch tool with deployment_id '{}'", accepted.deployment_id),
                )))
            }
        }

        "skoci_watch" => {
            let dep_id = match args.get("deployment_id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'deployment_id' is required",
                        "Provide deployment_id to watch",
                    )))
                }
            };

            let mut stream = client.follow_events(dep_id);
            let mut log_lines = Vec::new();
            let mut final_response: Option<DeployResponse> = None;
            let mut final_error: Option<SkociError> = None;

            while let Some(res) = stream.next().await {
                match res {
                    Ok(ev) => {
                        if ev.status == StageStatus::Log {
                            log_lines.push(ev.message);
                            if log_lines.len() > 50 {
                                log_lines.remove(0);
                            }
                        }
                        if ev.stage == DeployStage::Done {
                            if let Some(r) = ev.result {
                                final_response = Some(r);
                            }
                            if let Some(e) = ev.error {
                                final_error = Some(e);
                            }
                        }
                    }
                    Err(err) => {
                        final_error = Some(err);
                        break;
                    }
                }
            }

            if let Some(resp) = final_response {
                wrap_api_response(Ok(ApiResponse::ok(resp, "Deployment successful")))
            } else if let Some(mut err) = final_error {
                if !log_lines.is_empty() {
                    let tail_logs = log_lines.iter().rev().take(30).rev().cloned().collect::<Vec<_>>().join("\n");
                    err.message = format!("{}\n\nRecent build/container logs:\n{}", err.message, tail_logs);
                }
                wrap_api_response::<DeployResponse>(Err(err))
            } else {
                wrap_api_response::<DeployResponse>(Err(SkociError::internal(
                    "Event stream disconnected without final status",
                    format!("Use skoci_watch tool again with deployment_id '{}'", dep_id),
                )))
            }
        }

        "skoci_status" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            wrap_api_response(client.status(&app).await)
        }

        "skoci_logs" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let env = args.get("env").and_then(|v| v.as_str()).map(|s| s.to_string());
            let tail = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            wrap_api_response(client.logs(&app, LogsQuery { env, tail }).await)
        }

        "skoci_deployments" => {
            let app = resolve_mcp_app(&args, &default_path).ok();
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|l| l as usize);
            wrap_api_response(client.deployments(DeploymentsQuery { app, limit }).await)
        }

        "skoci_secret_set" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let key = match args.get("key").and_then(|v| v.as_str()) {
                Some(k) => k.to_string(),
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'key' is required",
                        "Specify secret key name",
                    )))
                }
            };
            let value = match args.get("value").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'value' is required",
                        "Specify secret value",
                    )))
                }
            };
            wrap_api_response(client.secret_set(SecretSetRequest { app, key, value }).await)
        }

        "skoci_secret_list" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            wrap_api_response(client.secret_list(&app).await)
        }

        "skoci_promote" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let reason = match args.get("reason").and_then(|v| v.as_str()) {
                Some(r) => r.to_string(),
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'reason' is required",
                        "Explain reason for production promotion",
                    )))
                }
            };
            let version = args.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            wrap_api_response(client.promote(PromoteRequest { app, reason, version }).await)
        }

        "skoci_rollback" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let env = args.get("env").and_then(|v| v.as_str()).unwrap_or("preview").to_string();
            let target_version = args.get("target_version").and_then(|v| v.as_str()).map(|s| s.to_string());
            wrap_api_response(client.rollback(RollbackRequest { app, env, target_version }).await)
        }

        "skoci_approvals" => {
            wrap_api_response(client.approvals().await)
        }

        "skoci_doctor" => {
            wrap_api_response(client.doctor().await)
        }

        "skoci_db_backup" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("db").to_string();
            wrap_api_response(client.db_backup(DbBackupRequest { app, service }).await)
        }

        "skoci_db_list" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            wrap_api_response(client.db_backups(&app).await)
        }

        "skoci_db_restore" => {
            let app = match resolve_mcp_app(&args, &default_path) {
                Ok(a) => a,
                Err(err) => return wrap_api_response::<()>(Err(err)),
            };
            let backup_id = match args.get("backup_id").and_then(|v| v.as_str()) {
                Some(b) => b.to_string(),
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'backup_id' is required",
                        "Specify backup ID to restore",
                    )))
                }
            };
            let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("db").to_string();
            wrap_api_response(client.db_restore(DbRestoreRequest { app, service, backup_id }).await)
        }

        _ => McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: format!("Tool '{name}' not found"),
            }],
            is_error: Some(true),
        },
    }
}

fn validate_mcp_path(path: &std::path::Path) -> Result<PathBuf, SkociError> {
    let cwd = std::env::current_dir().map_err(|e| {
        SkociError::bad_request(format!("Cannot determine workspace directory: {e}"), "Ensure workspace directory exists")
    })?;
    let canonical_cwd = cwd.canonicalize().unwrap_or(cwd);
    let resolved = if path.is_relative() {
        canonical_cwd.join(path)
    } else {
        path.to_path_buf()
    };
    let canonical_resolved = resolved.canonicalize().unwrap_or(resolved);
    if !canonical_resolved.starts_with(&canonical_cwd) {
        return Err(SkociError::bad_request(
            format!("Path '{}' escapes workspace root ('{}')", path.display(), canonical_cwd.display()),
            "Specify a relative path inside the workspace root directory",
        ));
    }
    Ok(canonical_resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_count_and_names() {
        let tools = list_tools();
        assert_eq!(tools.len(), 15);
        let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"skoci_init".to_string()));
        assert!(names.contains(&"skoci_deploy".to_string()));
        assert!(names.contains(&"skoci_watch".to_string()));
        assert!(names.contains(&"skoci_status".to_string()));
        assert!(names.contains(&"skoci_logs".to_string()));
        assert!(names.contains(&"skoci_deployments".to_string()));
        assert!(names.contains(&"skoci_secret_set".to_string()));
        assert!(names.contains(&"skoci_secret_list".to_string()));
        assert!(names.contains(&"skoci_promote".to_string()));
        assert!(names.contains(&"skoci_rollback".to_string()));
        assert!(names.contains(&"skoci_approvals".to_string()));
        assert!(names.contains(&"skoci_doctor".to_string()));
        assert!(names.contains(&"skoci_db_backup".to_string()));
        assert!(names.contains(&"skoci_db_list".to_string()));
        assert!(names.contains(&"skoci_db_restore".to_string()));
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let client = SkociClient::new("http://localhost:8787", None, None).unwrap();
        let res = call_tool(&client, "unknown_tool", json!({})).await;
        assert_eq!(res.is_error, Some(true));
        assert!(res.content[0].text.contains("not found"));
    }
}
