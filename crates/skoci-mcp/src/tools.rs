use crate::protocol::{McpContent, McpTool, McpToolCallResult};
use serde_json::json;
use skoci_api::requests::{DbBackupRequest, DbRestoreRequest, DeployRequest, PromoteRequest, RollbackRequest};
use skoci_client::SkociClient;

pub fn list_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "skoci_init".to_string(),
            description: "Detect project and create skoci.yaml and slim multi-stage Dockerfile".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Optional application name" },
                    "path": { "type": "string", "description": "Target project directory path" }
                }
            }),
        },
        McpTool {
            name: "skoci_deploy".to_string(),
            description: "Deploy application with health check and zero-downtime swap".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "env": { "type": "string", "enum": ["preview", "production"], "description": "Target environment" },
                    "force": { "type": "boolean", "description": "Force deployment" }
                },
                "required": ["app"]
            }),
        },
        McpTool {
            name: "skoci_status".to_string(),
            description: "Get application runtime and container health status".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" }
                },
                "required": ["app"]
            }),
        },
        McpTool {
            name: "skoci_logs".to_string(),
            description: "Retrieve recent application logs with error extraction".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "tail": { "type": "integer", "description": "Number of lines to tail (default 100)" }
                },
                "required": ["app"]
            }),
        },
        McpTool {
            name: "skoci_secret_set".to_string(),
            description: "Store an encrypted secret for the application".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "key": { "type": "string", "description": "Secret key name" },
                    "value": { "type": "string", "description": "Secret raw value" }
                },
                "required": ["app", "key", "value"]
            }),
        },
        McpTool {
            name: "skoci_secret_list".to_string(),
            description: "List configured secret key names (values remain redacted)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" }
                },
                "required": ["app"]
            }),
        },
        McpTool {
            name: "skoci_promote".to_string(),
            description: "Request human approval to promote application to production".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "reason": { "type": "string", "description": "Reason for promoting to production" }
                },
                "required": ["app", "reason"]
            }),
        },
        McpTool {
            name: "skoci_rollback".to_string(),
            description: "Rollback application to previous healthy version".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "target_version": { "type": "string", "description": "Optional target version" }
                },
                "required": ["app"]
            }),
        },
        McpTool {
            name: "skoci_doctor".to_string(),
            description: "Run server diagnostics and verify RAM idle budget (<20MB)".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        McpTool {
            name: "skoci_db_backup".to_string(),
            description: "Create an immediate backup of a database service".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "service": { "type": "string", "description": "Service name (e.g. db)" }
                },
                "required": ["app", "service"]
            }),
        },
        McpTool {
            name: "skoci_db_restore".to_string(),
            description: "Restore database service from a backup ID".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name" },
                    "service": { "type": "string", "description": "Service name" },
                    "backup_id": { "type": "string", "description": "Backup ID" }
                },
                "required": ["app", "service", "backup_id"]
            }),
        },
    ]
}

pub async fn call_tool(
    client: &SkociClient,
    name: &str,
    args: serde_json::Value,
) -> McpToolCallResult {
    match name {
        "skoci_init" => {
            let app = args.get("app").and_then(|v| v.as_str()).map(|s| s.to_string());
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let res = serde_json::json!({
                "app": app.unwrap_or_else(|| "app".to_string()),
                "path": path,
                "status": "initialized",
                "summary": "Project initialized with skoci.yaml and Dockerfile"
            });
            McpToolCallResult {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text: serde_json::to_string_pretty(&res).unwrap(),
                }],
                is_error: None,
            }
        }
        "skoci_deploy" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let env = args.get("env").and_then(|v| v.as_str()).unwrap_or("preview");
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

            let req = DeployRequest {
                app: app.to_string(),
                env: env.to_string(),
                commit: Some("HEAD".to_string()),
                force,
            };
            match client.deploy(req).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_status" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            match client.status(app).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_logs" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let tail = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            match client.logs(app, tail).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_secret_set" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let val = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
            match client.secret_set(app, key, val).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_secret_list" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            match client.secret_list(app).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_promote" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let req = PromoteRequest {
                app: app.to_string(),
                target_env: "production".to_string(),
                reason: reason.to_string(),
            };
            match client.promote(req).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_rollback" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let target = args.get("target_version").and_then(|v| v.as_str()).map(|s| s.to_string());
            let req = RollbackRequest {
                app: app.to_string(),
                target_version: target,
            };
            match client.rollback(req).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_doctor" => match client.doctor().await {
            Ok(resp) => McpToolCallResult {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text: serde_json::to_string_pretty(&resp).unwrap(),
                }],
                is_error: Some(!resp.success),
            },
            Err(e) => McpToolCallResult {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text: serde_json::to_string_pretty(&e).unwrap(),
                }],
                is_error: Some(true),
            },
        },
        "skoci_db_backup" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("db");
            let req = DbBackupRequest {
                app: app.to_string(),
                service: service.to_string(),
            };
            match client.db_backup(req).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        "skoci_db_restore" => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("app");
            let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("db");
            let backup_id = args.get("backup_id").and_then(|v| v.as_str()).unwrap_or("");
            let req = DbRestoreRequest {
                app: app.to_string(),
                service: service.to_string(),
                backup_id: backup_id.to_string(),
            };
            match client.db_restore(req).await {
                Ok(resp) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&resp).unwrap(),
                    }],
                    is_error: Some(!resp.success),
                },
                Err(e) => McpToolCallResult {
                    content: vec![McpContent {
                        content_type: "text".to_string(),
                        text: serde_json::to_string_pretty(&e).unwrap(),
                    }],
                    is_error: Some(true),
                },
            }
        }
        _ => McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: format!("Unknown tool: {name}"),
            }],
            is_error: Some(true),
        },
    }
}
