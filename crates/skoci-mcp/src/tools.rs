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
                .unwrap_or_else(|e| format!("Gagal memformat JSON: {e}"));
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
                .unwrap_or_else(|e| format!("Gagal memformat JSON: {e}"));
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
            description: "Deteksi framework proyek dan buat skoci.yaml, Dockerfile, serta .dockerignore".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Direktori proyek (default: .)" },
                    "app": { "type": "string", "description": "Nama aplikasi opsional" }
                }
            }),
        },
        McpTool {
            name: "skoci_deploy".to_string(),
            description: "Deploy proyek ke server Skoci (pack, upload, dan pantau event sampai selesai)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Direktori proyek (default: .)" },
                    "env": { "type": "string", "description": "Target environment (preview atau production, default: preview)" }
                }
            }),
        },
        McpTool {
            name: "skoci_watch".to_string(),
            description: "Pantau deployment yang sedang berjalan hingga selesai dan kembalikan status akhir".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "deployment_id": { "type": "string", "description": "ID deployment yang dipantau" }
                },
                "required": ["deployment_id"]
            }),
        },
        McpTool {
            name: "skoci_status".to_string(),
            description: "Periksa status kontainer, kesehatan live, dan URL aplikasi".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_logs".to_string(),
            description: "Lihat log aplikasi terbaru beserta ringkasan baris error".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "env": { "type": "string", "description": "Environment aplikasi (opsional)" },
                    "tail": { "type": "number", "description": "Jumlah baris log terbaru (default: 100)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_deployments".to_string(),
            description: "Daftar riwayat deployment aplikasi".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi opsional" },
                    "limit": { "type": "number", "description": "Batas jumlah riwayat (default: 20)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_secret_set".to_string(),
            description: "Simpan secret terenkripsi untuk aplikasi".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "key": { "type": "string", "description": "Nama kunci secret (huruf besar dan underscore)" },
                    "value": { "type": "string", "description": "Nilai secret" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                },
                "required": ["key", "value"]
            }),
        },
        McpTool {
            name: "skoci_secret_list".to_string(),
            description: "Daftar nama kunci secret aplikasi (nilai tetap terenkripsi dan dirahasiakan)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_promote".to_string(),
            description: "Ajukan promosi versi preview ke production (memerlukan persetujuan admin jika dipanggil oleh agent)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "reason": { "type": "string", "description": "Alasan promosi" },
                    "version": { "type": "string", "description": "Versi yang akan dipromosikan (opsional)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                },
                "required": ["reason"]
            }),
        },
        McpTool {
            name: "skoci_rollback".to_string(),
            description: "Rollback aplikasi ke versi sehat sebelumnya".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "env": { "type": "string", "description": "Target environment (default: preview)" },
                    "target_version": { "type": "string", "description": "Versi target rollback opsional" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_approvals".to_string(),
            description: "Lihat daftar permintaan persetujuan yang menunggu tindakan admin".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "skoci_doctor".to_string(),
            description: "Periksa kesehatan server Skoci, daemon Docker, RAM, dan status proxy".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "skoci_db_backup".to_string(),
            description: "Buat backup database aplikasi secara aman".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "service": { "type": "string", "description": "Nama service database (default: db)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_db_list".to_string(),
            description: "Daftar riwayat backup database untuk aplikasi".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
                }
            }),
        },
        McpTool {
            name: "skoci_db_restore".to_string(),
            description: "Ajukan restore database dari file backup (memerlukan persetujuan admin jika dipanggil oleh agent)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Nama aplikasi (opsional jika skoci.yaml ada)" },
                    "backup_id": { "type": "string", "description": "ID backup database" },
                    "service": { "type": "string", "description": "Nama service database (default: db)" },
                    "path": { "type": "string", "description": "Direktori tempat skoci.yaml berada (default: .)" }
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
    let default_path = PathBuf::from(
        args.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("."),
    );

    match name {
        "skoci_init" => {
            let app = args.get("app").and_then(|v| v.as_str());
            let res = skoci_cli::init_project(&default_path, app).map(|(r, _)| r);
            wrap_api_response(res.map(|r| ApiResponse::ok(r, "Proyek berhasil diinisialisasi")))
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

            let out = tokio::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(&default_path)
                .output()
                .await;
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
                                "Respons kosong dari server",
                                "Coba deploy kembali",
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
                wrap_api_response(Ok(ApiResponse::ok(resp, "Deployment berhasil")))
            } else if let Some(mut err) = final_error {
                if !log_lines.is_empty() {
                    let tail_logs = log_lines.iter().rev().take(30).rev().cloned().collect::<Vec<_>>().join("\n");
                    err.message = format!("{}\n\nLog build/kontainer terakhir:\n{}", err.message, tail_logs);
                }
                wrap_api_response::<DeployResponse>(Err(err))
            } else {
                wrap_api_response::<DeployResponse>(Err(SkociError::internal(
                    "Event stream terputus tanpa data status akhir",
                    format!("Gunakan tool skoci_watch dengan deployment_id '{}'", accepted.deployment_id),
                )))
            }
        }

        "skoci_watch" => {
            let dep_id = match args.get("deployment_id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'deployment_id' wajib diisi",
                        "Berikan deployment_id yang ingin dipantau",
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
                wrap_api_response(Ok(ApiResponse::ok(resp, "Deployment berhasil")))
            } else if let Some(mut err) = final_error {
                if !log_lines.is_empty() {
                    let tail_logs = log_lines.iter().rev().take(30).rev().cloned().collect::<Vec<_>>().join("\n");
                    err.message = format!("{}\n\nLog build/kontainer terakhir:\n{}", err.message, tail_logs);
                }
                wrap_api_response::<DeployResponse>(Err(err))
            } else {
                wrap_api_response::<DeployResponse>(Err(SkociError::internal(
                    "Event stream terputus tanpa status akhir",
                    format!("Gunakan tool skoci_watch kembali dengan deployment_id '{}'", dep_id),
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
                        "Parameter 'key' wajib diisi",
                        "Tentukan nama kunci secret",
                    )))
                }
            };
            let value = match args.get("value").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => {
                    return wrap_api_response::<()>(Err(SkociError::bad_request(
                        "Parameter 'value' wajib diisi",
                        "Tentukan nilai secret",
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
                        "Parameter 'reason' wajib diisi",
                        "Jelaskan alasan promosi ke production",
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
                        "Parameter 'backup_id' wajib diisi",
                        "Tentukan ID backup yang ingin direstore",
                    )))
                }
            };
            let service = args.get("service").and_then(|v| v.as_str()).unwrap_or("db").to_string();
            wrap_api_response(client.db_restore(DbRestoreRequest { app, service, backup_id }).await)
        }

        _ => McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: format!("Tool '{name}' tidak ditemukan"),
            }],
            is_error: Some(true),
        },
    }
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
        let client = SkociClient::new("http://localhost:8787", None);
        let res = call_tool(&client, "unknown_tool", json!({})).await;
        assert_eq!(res.is_error, Some(true));
        assert!(res.content[0].text.contains("tidak ditemukan"));
    }
}
