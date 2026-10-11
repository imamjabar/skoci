use crate::args::{Commands, DbCommands, SecretCommands, TokenCommands};
use crate::output::print_result;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{ApproveRequest, DbBackupRequest, DbRestoreRequest, DeployRequest, PromoteRequest, RollbackRequest};
use skoci_api::responses::InitResponse;
use skoci_client::SkociClient;
use skoci_core::config::ConfigLoader;
use skoci_core::detector::ProjectDetector;
use std::path::Path;

pub async fn run_command(cmd: Commands, server_url: &str, token: Option<String>, json_mode: bool) {
    let client = SkociClient::new(server_url, token);

    match cmd {
        Commands::Init { app, path } => {
            let res = execute_init(app, &path);
            print_result(json_mode, res);
        }
        Commands::Deploy { config, env, force } => {
            let app_name = resolve_app_name(config.as_deref()).unwrap_or_else(|_| "default-app".to_string());
            let req = DeployRequest {
                app: app_name,
                env,
                commit: Some("HEAD".to_string()),
                force,
            };
            let res = client.deploy(req).await;
            print_result(json_mode, res);
        }
        Commands::Status { app } => {
            let app_name = app.or_else(|| resolve_app_name(None).ok()).unwrap_or_else(|| "default-app".to_string());
            let res = client.status(&app_name).await;
            print_result(json_mode, res);
        }
        Commands::Logs { app, tail } => {
            let app_name = app.or_else(|| resolve_app_name(None).ok()).unwrap_or_else(|| "default-app".to_string());
            let res = client.logs(&app_name, tail).await;
            print_result(json_mode, res);
        }
        Commands::Secret { command } => match command {
            SecretCommands::Set { key, value } => {
                let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
                let res = client.secret_set(&app_name, &key, &value).await;
                print_result(json_mode, res);
            }
            SecretCommands::List => {
                let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
                let res = client.secret_list(&app_name).await;
                print_result(json_mode, res);
            }
        },
        Commands::Promote { reason, env } => {
            let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
            let req = PromoteRequest {
                app: app_name,
                target_env: env,
                reason,
            };
            let res = client.promote(req).await;
            print_result(json_mode, res);
        }
        Commands::Approve { request_id } => {
            let req = ApproveRequest {
                request_id,
                note: None,
            };
            let res = client.approve(req).await;
            print_result(json_mode, res);
        }
        Commands::Rollback { target } => {
            let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
            let req = RollbackRequest {
                app: app_name,
                target_version: target,
            };
            let res = client.rollback(req).await;
            print_result(json_mode, res);
        }
        Commands::Doctor => {
            let res = client.doctor().await;
            print_result(json_mode, res);
        }
        Commands::Db { command } => match command {
            DbCommands::Backup { service } => {
                let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
                let req = DbBackupRequest {
                    app: app_name,
                    service,
                };
                let res = client.db_backup(req).await;
                print_result(json_mode, res);
            }
            DbCommands::Restore { backup_id, service } => {
                let app_name = resolve_app_name(None).unwrap_or_else(|_| "default-app".to_string());
                let req = DbRestoreRequest {
                    app: app_name,
                    service,
                    backup_id,
                };
                let res = client.db_restore(req).await;
                print_result(json_mode, res);
            }
        },
        Commands::Token { command } => match command {
            TokenCommands::Create { app, name } => {
                let res = client.token_create(&app, &name).await;
                print_result(json_mode, res);
            }
            TokenCommands::List { app } => {
                let res = client.token_list(&app).await;
                print_result(json_mode, res);
            }
        },
        Commands::AgentGuide => {
            let guide = r#"# Skoci Agent Guide

## Instructions for AI Coding Agents:
1. When deploying: always invoke `skoci deploy --json`.
2. Check `{ "success": true, "data": { ... } }`.
3. On error `{ "code": "...", "message": "...", "fix": "..." }`:
   - Read `fix` and execute the suggested corrective action.
   - If code is `approval_required`: do NOT retry repeatedly. Report the `request_id` to the human user.
4. Secrets: never write plaintext credentials to `skoci.yaml`.
   - Use `skoci secret set <KEY> <VALUE> --json`.
5. Rollback: if smoke test fails post-deploy, run `skoci rollback --json`.
"#;
            if json_mode {
                let res = serde_json::json!({
                    "success": true,
                    "summary": "Panduan agen deployment Skoci",
                    "guide": guide
                });
                println!("{}", serde_json::to_string_pretty(&res).unwrap());
            } else {
                println!("{guide}");
            }
        }
        Commands::InstallService { service_name, bind } => {
            let current_exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/skoci"));
            let unit = format!(
                r#"[Unit]
Description=Skoci Deployment Platform
After=network.target docker.service
Requires=docker.service

[Service]
Type=simple
User=root
ExecStart={} server --bind {}
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
"#,
                current_exe.display(),
                bind
            );
            println!("=== Systemd Unit File (/etc/systemd/system/{service_name}.service) ===");
            println!("{unit}");
            println!("Untuk memasang di server Linux:");
            println!("  echo '<unit di atas>' | sudo tee /etc/systemd/system/{service_name}.service");
            println!("  sudo systemctl daemon-reload && sudo systemctl enable --now {service_name}");
        }
    }
}


fn resolve_app_name(config_path: Option<&str>) -> SkociResult<String> {
    let candidates = if let Some(p) = config_path {
        vec![p.to_string()]
    } else {
        vec![
            "skoci.yaml".to_string(),
            ".skoci/skoci.yaml".to_string(),
        ]
    };

    for p in candidates {
        if let Ok(config) = ConfigLoader::load_config(&p) {
            return Ok(config.app);
        }
    }

    let current_dir = std::env::current_dir().map_err(|e| {
        SkociError::bad_request(e.to_string(), "Cek direktori kerja")
    })?;
    let name = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app")
        .to_lowercase()
        .replace(' ', "-");
    Ok(name)
}

fn execute_init(app: Option<String>, path: &str) -> SkociResult<ApiResponse<InitResponse>> {
    let dir = Path::new(path);
    let app_name = app.unwrap_or_else(|| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app")
            .to_lowercase()
            .replace(' ', "-")
    });

    let framework = ProjectDetector::detect(dir);
    let mut created_files = Vec::new();

    let yaml_path = dir.join("skoci.yaml");
    if !yaml_path.exists() {
        let yaml_content = ProjectDetector::generate_skoci_yaml(&app_name, &framework);
        std::fs::write(&yaml_path, yaml_content).map_err(|e| {
            SkociError::bad_request(format!("Gagal menulis skoci.yaml: {e}"), "Cek permissions")
        })?;
        created_files.push("skoci.yaml".to_string());
    }

    let policy_path = dir.join("skoci.policy.yaml");
    if !policy_path.exists() {
        let policy_content = ProjectDetector::generate_policy_yaml();
        std::fs::write(&policy_path, policy_content).map_err(|e| {
            SkociError::bad_request(format!("Gagal menulis skoci.policy.yaml: {e}"), "Cek permissions")
        })?;
        created_files.push("skoci.policy.yaml".to_string());
    }

    let dockerfile_path = dir.join("Dockerfile");
    if !dockerfile_path.exists() {
        let dockerfile_content = framework.default_dockerfile();
        std::fs::write(&dockerfile_path, dockerfile_content).map_err(|e| {
            SkociError::bad_request(format!("Gagal menulis Dockerfile: {e}"), "Cek permissions")
        })?;
        created_files.push("Dockerfile".to_string());
    }

    let data = InitResponse {
        detected_framework: framework.name().to_string(),
        created_files,
        app_name: app_name.clone(),
    };

    Ok(ApiResponse::ok(
        data,
        format!("Inisialisasi berhasil untuk framework {}", framework.name()),
    ))
}
