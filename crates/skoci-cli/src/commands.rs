use futures_util::StreamExt;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{
    ApproveRequest, DbBackupRequest, DbRestoreRequest, DefaultAppRequest, DeployRequest,
    DeploymentsQuery, LogsQuery, PromoteRequest, RollbackRequest, SecretSetRequest,
    TokenCreateRequest,
};
use skoci_api::responses::{
    DeployResponse, DeployStage, InitResponse, StageStatus,
};
use skoci_client::SkociClient;
use skoci_core::config::ConfigLoader;
use skoci_core::detector::ProjectDetector;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Instant;

use crate::args::{AppCommands, Commands, DbCommands, SecretCommands, TokenCommands};
use crate::output;

pub fn error_exit_code(err: &SkociError) -> i32 {
    match err.code.as_str() {
        "unauthorized" | "scope_violation" | "rate_limited" => 3,
        "approval_required" => 2,
        _ => 1,
    }
}

pub fn resolve_app_in(path: &Path, provided: Option<&str>) -> SkociResult<String> {
    if let Some(app) = provided {
        let trimmed = app.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let config_path = path.join("skoci.yaml");
    if config_path.exists() {
        if let Ok(cfg) = ConfigLoader::load_config(&config_path) {
            let trimmed = cfg.app.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    Err(SkociError::config_invalid(
        "Application name not found",
        "Specify the application name via arguments or run `skoci init` to create skoci.yaml",
    ))
}

pub fn resolve_app(provided: Option<String>) -> SkociResult<String> {
    resolve_app_in(Path::new("."), provided.as_deref())
}

pub fn init_project(path: impl AsRef<Path>, app_override: Option<&str>) -> SkociResult<InitResponse> {
    let path = path.as_ref();
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| {
            SkociError::internal(
                format!("Failed to create directory {}: {e}", path.display()),
                "Ensure the directory can be created and permissions are sufficient",
            )
        })?;
    }

    let app_name = if let Some(app) = app_override {
        app.trim().to_string()
    } else {
        path.canonicalize()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().replace(['_', ' '], "-"))
            })
            .unwrap_or_else(|| "app".to_string())
    };

    let detection = ProjectDetector::detect(path);
    let mut created_files = Vec::new();
    let mut skipped_files = Vec::new();

    let yaml_file = path.join("skoci.yaml");
    if !yaml_file.exists() {
        let content = ProjectDetector::generate_skoci_yaml(&app_name, &detection);
        std::fs::write(&yaml_file, content).map_err(|e| {
            SkociError::internal(
                format!("Failed to write skoci.yaml: {e}"),
                "Check directory write permissions",
            )
        })?;
        created_files.push("skoci.yaml".to_string());
    } else {
        skipped_files.push("skoci.yaml".to_string());
    }

    if let Some(df_content) = ProjectDetector::dockerfile(&detection) {
        let df_file = path.join("Dockerfile");
        if !df_file.exists() {
            std::fs::write(&df_file, df_content).map_err(|e| {
                SkociError::internal(
                    format!("Failed to write Dockerfile: {e}"),
                    "Check directory write permissions",
                )
            })?;
            created_files.push("Dockerfile".to_string());
        } else {
            skipped_files.push("Dockerfile".to_string());
        }
    }

    let di_file = path.join(".dockerignore");
    if !di_file.exists() {
        let di_content = ProjectDetector::dockerignore();
        std::fs::write(&di_file, di_content).map_err(|e| {
            SkociError::internal(
                format!("Failed to write .dockerignore: {e}"),
                "Check directory write permissions",
            )
        })?;
        created_files.push(".dockerignore".to_string());
    } else {
        skipped_files.push(".dockerignore".to_string());
    }

    let gitignore_file = path.join(".gitignore");
    let (needs_env_skoci, add_newline) = if gitignore_file.exists() {
        let content = std::fs::read_to_string(&gitignore_file).unwrap_or_default();
        let needs = !content.lines().any(|l| l.trim() == ".env.skoci");
        let newline = !content.is_empty() && !content.ends_with('\n');
        (needs, newline)
    } else {
        (true, false)
    };

    if needs_env_skoci {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_file)
            .map_err(|e| {
                SkociError::internal(
                    format!("Failed to update .gitignore: {e}"),
                    "Check write permissions for .gitignore",
                )
            })?;
        if add_newline {
            let _ = writeln!(f);
        }
        let _ = writeln!(f, ".env.skoci");
    }

    Ok(InitResponse {
        detected_framework: detection.framework.name().to_string(),
        created_files,
        app_name,
        skipped_files,
        notes: detection.notes,
    })
}

fn step_number(stage: DeployStage) -> Option<usize> {
    DeployStage::STEPS.iter().position(|&s| s == stage).map(|idx| idx + 1)
}

pub async fn stream_events_and_render(
    client: &SkociClient,
    deployment_id: &str,
    _app: &str,
    _env: &str,
    version: Option<&str>,
    is_json: bool,
) -> i32 {
    let mut stream = client.follow_events(deployment_id);
    let is_term = std::io::stderr().is_terminal();
    let start_time = Instant::now();

    if !is_json && version.is_none() {
        eprintln!("▸ Watching deployment: {}", deployment_id);
    }

    let mut accumulated_logs = Vec::new();
    let mut log_window: Vec<String> = Vec::new();
    let mut active_stage: Option<(usize, &'static str)> = None;
    let mut rendered_lines: usize = 0;
    let mut final_response: Option<DeployResponse> = None;
    let mut final_error: Option<SkociError> = None;

    while let Some(res) = stream.next().await {
        match res {
            Ok(event) => {
                if is_json {
                    if let Some(r) = event.result {
                        final_response = Some(r);
                    }
                    if let Some(e) = event.error {
                        final_error = Some(e);
                    }
                    continue;
                }

                if event.status == StageStatus::Log {
                    accumulated_logs.push(event.message.clone());
                    if accumulated_logs.len() > 100 {
                        accumulated_logs.remove(0);
                    }
                    if let (true, Some((n, label))) = (is_term, active_stage) {
                        let truncated = if event.message.chars().count() > 100 {
                            let mut s: String = event.message.chars().take(97).collect();
                            s.push_str("...");
                            s
                        } else {
                            event.message.clone()
                        };
                        log_window.push(truncated);
                        if log_window.len() > 5 {
                            log_window.remove(0);
                        }
                        use std::io::Write;
                        let mut out = std::io::stderr().lock();
                        for _ in 0..rendered_lines {
                            let _ = write!(out, "\x1b[1A\x1b[2K\r");
                        }
                        let _ = writeln!(out, "  \x1b[36m▸ [{}/8] {}\x1b[0m", n, label);
                        for l in &log_window {
                            let _ = writeln!(out, "    \x1b[2m│ {}\x1b[0m", l);
                        }
                        let _ = out.flush();
                        rendered_lines = 1 + log_window.len();
                    }
                } else if let Some(n) = step_number(event.stage) {
                    let label = event.stage.label();
                    match event.status {
                        StageStatus::Running => {
                            if is_term {
                                use std::io::Write;
                                let mut out = std::io::stderr().lock();
                                for _ in 0..rendered_lines {
                                    let _ = write!(out, "\x1b[1A\x1b[2K\r");
                                }
                                let _ = writeln!(out, "  \x1b[36m▸ [{}/8] {}\x1b[0m", n, label);
                                let _ = out.flush();
                                log_window.clear();
                                active_stage = Some((n, label));
                                rendered_lines = 1;
                            }
                        }
                        StageStatus::Ok => {
                            if is_term {
                                use std::io::Write;
                                let mut out = std::io::stderr().lock();
                                for _ in 0..rendered_lines {
                                    let _ = write!(out, "\x1b[1A\x1b[2K\r");
                                }
                                let _ = writeln!(out, "  \x1b[32m✓ [{}/8] {}\x1b[0m", n, label);
                                let _ = out.flush();
                                log_window.clear();
                                active_stage = None;
                                rendered_lines = 0;
                            } else {
                                eprintln!("  ✓ [{}/8] {}", n, label);
                            }
                        }
                        StageStatus::Skipped => {
                            if is_term {
                                use std::io::Write;
                                let mut out = std::io::stderr().lock();
                                for _ in 0..rendered_lines {
                                    let _ = write!(out, "\x1b[1A\x1b[2K\r");
                                }
                                let _ = writeln!(out, "  \x1b[33m– [{}/8] {} (skipped)\x1b[0m", n, label);
                                let _ = out.flush();
                                log_window.clear();
                                active_stage = None;
                                rendered_lines = 0;
                            } else {
                                eprintln!("  – [{}/8] {} (skipped)", n, label);
                            }
                        }
                        StageStatus::Failed => {
                            let detail = if event.message.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", event.message)
                            };
                            if is_term {
                                use std::io::Write;
                                let mut out = std::io::stderr().lock();
                                for _ in 0..rendered_lines {
                                    let _ = write!(out, "\x1b[1A\x1b[2K\r");
                                }
                                let _ = writeln!(out, "  \x1b[31m✗ [{}/8] {}{}\x1b[0m", n, label, detail);
                                let _ = out.flush();
                                log_window.clear();
                                active_stage = None;
                                rendered_lines = 0;
                            } else {
                                eprintln!("  ✗ [{}/8] {}{}", n, label, detail);
                            }
                        }
                        _ => {}
                    }
                }

                if event.stage == DeployStage::Done {
                    if is_term && rendered_lines > 0 {
                        use std::io::Write;
                        let mut out = std::io::stderr().lock();
                        for _ in 0..rendered_lines {
                            let _ = write!(out, "\x1b[1A\x1b[2K\r");
                        }
                        let _ = out.flush();
                        rendered_lines = 0;
                    }
                    if let Some(deploy_res) = event.result {
                        final_response = Some(deploy_res);
                    }
                    if let Some(err) = event.error {
                        final_error = Some(err);
                    }
                }
            }
            Err(err) => {
                final_error = Some(err);
                break;
            }
        }
    }

    if is_json {
        if let Some(resp) = final_response {
            output::print_json(&ApiResponse::ok(resp, "Deployment successful"));
            return 0;
        } else if let Some(err) = final_error {
            output::print_error(&err, true);
            return error_exit_code(&err);
        } else {
            let err = SkociError::internal(
                "Event stream disconnected without final result data",
                format!("Use `skoci watch {}` to check status", deployment_id),
            );
            output::print_error(&err, true);
            return error_exit_code(&err);
        }
    }

    if let Some(resp) = final_response {
        let dur = start_time.elapsed().as_secs_f64();
        for url in &resp.urls {
            if is_term {
                println!("\x1b[1;32m✓ Live:\x1b[0m {}", url);
            } else {
                println!("✓ Live: {}", url);
            }
        }
        println!("Version: {}  Duration: {:.1}s", resp.version, dur);
        println!("\nHints:");
        println!("  skoci logs {}", resp.app);
        println!("  skoci status {}", resp.app);
        0
    } else if let Some(err) = final_error {
        if !accumulated_logs.is_empty() {
            eprintln!("\nLast log summary:");
            for line in accumulated_logs.iter().rev().take(30).rev() {
                eprintln!("  │ {}", line);
            }
        }
        if is_term {
            eprintln!("\n\x1b[1;31m✗ Deployment failed:\x1b[0m {}", err.message);
        } else {
            eprintln!("\n✗ Deployment failed: {}", err.message);
        }
        eprintln!("Fix: {}", err.fix);
        eprintln!("Deployment ID: {}  (skoci watch {})", deployment_id, deployment_id);
        error_exit_code(&err)
    } else {
        eprintln!("\n✗ Stream completed without final deployment status");
        eprintln!("Fix: Use `skoci watch {}` to reconnect", deployment_id);
        1
    }
}

pub async fn run_command(
    cmd: Commands,
    server_url: &str,
    token: Option<String>,
    json: bool,
) -> i32 {
    let client = SkociClient::new(server_url, token);

    match cmd {
        Commands::Connect { server, token: conn_token, insecure } => {
            let server_lower = server.to_lowercase();
            if server_lower.starts_with("http://")
                && !server_lower.contains("localhost")
                && !server_lower.contains("127.0.0.1")
                && !insecure
            {
                let err = SkociError::bad_request(
                    format!(
                        "Server URL uses unencrypted HTTP protocol ('{server}'). Token will be sent in plaintext."
                    ),
                    "Use HTTPS ('https://...'), an SSH tunnel (localhost), or add the --insecure flag if you understand the risks.",
                );
                output::print_error(&err, json);
                return 1;
            }

            let env_content = format!("SKOCI_SERVER_URL={}\nSKOCI_TOKEN={}\n", server, conn_token);
            if let Err(e) = std::fs::write(".env.skoci", &env_content) {
                let err = SkociError::internal(
                    format!("Failed to write .env.skoci: {e}"),
                    "Check write permissions in the current directory",
                );
                output::print_error(&err, json);
                return 1;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(".env.skoci", std::fs::Permissions::from_mode(0o600));
            }

            let gitignore_path = Path::new(".gitignore");
            let (needs_entry, add_newline) = if gitignore_path.exists() {
                let content = std::fs::read_to_string(gitignore_path).unwrap_or_default();
                let needs = !content.lines().any(|l| l.trim() == ".env.skoci");
                let newline = !content.is_empty() && !content.ends_with('\n');
                (needs, newline)
            } else {
                (true, false)
            };

            if needs_entry {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(gitignore_path) {
                    if add_newline {
                        let _ = writeln!(f);
                    }
                    let _ = writeln!(f, ".env.skoci");
                }
            }

            let test_client = SkociClient::new(&server, Some(conn_token));
            match test_client.doctor().await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else {
                        println!("✓ Successfully connected and saved configuration to .env.skoci (mode 0600)\n");
                        if let Some(doc) = &resp.data {
                            output::render_doctor(doc);
                        }
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Init { app, path } => {
            match init_project(&path, app.as_deref()) {
                Ok(res) => {
                    if json {
                        output::print_json(&ApiResponse::ok(res, "Project successfully initialized"));
                    } else {
                        println!("✓ Project '{}' successfully initialized!\n", res.app_name);
                        println!("Detected framework   : {}", res.detected_framework);
                        println!("Files created        : {}", res.created_files.join(", "));
                        if !res.skipped_files.is_empty() {
                            println!("Files skipped (exist): {}", res.skipped_files.join(", "));
                        }
                        if !res.notes.is_empty() {
                            println!("\nNotes:");
                            for note in &res.notes {
                                println!("  • {}", note);
                            }
                        }
                        println!("\nNext steps:");
                        println!("  1. Review skoci.yaml (port, health path, domains)");
                        println!("  2. Connect once: skoci connect --server <url> --token <token>");
                        println!("  3. Deploy: skoci deploy");
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Deploy { env, path, detach, commit } => {
            let config_path = path.join("skoci.yaml");
            let cfg = match ConfigLoader::load_config(&config_path) {
                Ok(c) => c,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };
            let app = cfg.app;

            let commit_val = match commit {
                Some(c) => Some(c),
                None => {
                    let out = tokio::process::Command::new("git")
                        .args(["rev-parse", "--short", "HEAD"])
                        .current_dir(&path)
                        .output()
                        .await;
                    match &out {
                        Ok(o) if o.status.success() => {
                            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                        }
                        _ => Some("none".to_string()),
                    }
                }
            };

            let packed = match skoci_client::pack_context(&path) {
                Ok(p) => p,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            let req = DeployRequest {
                app: app.clone(),
                env: env.clone(),
                commit: commit_val,
            };

            let start_upload = Instant::now();
            if !json {
                eprintln!("▸ Deploy {} → {}", app, env);
            }

            let resp = client.deploy_upload(&packed.path, &req, move |_sent, _total| {}).await;

            match resp {
                Ok(api_resp) => {
                    if !api_resp.success {
                        if let Some(err) = &api_resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }

                    let accepted = match api_resp.data {
                        Some(d) => d,
                        None => {
                            let err = SkociError::internal("Empty response from server", "Try deploying again");
                            output::print_error(&err, json);
                            return 1;
                        }
                    };

                    if !json {
                        let elapsed = start_upload.elapsed();
                        let mb = (packed.bytes as f64) / 1_048_576.0;
                        eprintln!(
                            "✓ Uploaded {:.1} MB ({} files) in {:.1}s",
                            mb,
                            packed.files,
                            elapsed.as_secs_f64()
                        );
                        eprintln!("  Version: {}  ID: {}", accepted.version, accepted.deployment_id);
                    }

                    if detach {
                        if json {
                            output::print_json(&ApiResponse::ok(
                                accepted,
                                "Deployment accepted in background",
                            ));
                        } else {
                            println!("\nDeployment running in background (detached).");
                            println!("Deployment ID: {}", accepted.deployment_id);
                            println!("Version      : {}", accepted.version);
                            println!("Use `skoci watch {}` to monitor status.", accepted.deployment_id);
                        }
                        0
                    } else {
                        stream_events_and_render(
                            &client,
                            &accepted.deployment_id,
                            &app,
                            &env,
                            Some(&accepted.version),
                            json,
                        )
                        .await
                    }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Watch { deployment_id } => {
            stream_events_and_render(&client, &deployment_id, "app", "env", None, json).await
        }

        Commands::Status { app } => {
            let app_name = match resolve_app(app) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            match client.status(&app_name).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else if let Some(data) = &resp.data {
                        output::render_status(data);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Deployments { app, limit } => {
            let app_name = app.or_else(|| resolve_app(None).ok());
            let query = DeploymentsQuery {
                app: app_name,
                limit: Some(limit),
            };

            match client.deployments(query).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else if let Some(data) = &resp.data {
                        output::render_deployments(data);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Logs { app, env, tail } => {
            let app_name = match resolve_app(app) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };
            let query = LogsQuery { env, tail };

            match client.logs(&app_name, query).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else if let Some(data) = &resp.data {
                        output::render_logs(data);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Secret { action } => {
            let app_name = match resolve_app(None) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            match action {
                SecretCommands::Set { key, value } => {
                    let req = SecretSetRequest {
                        app: app_name.clone(),
                        key: key.clone(),
                        value,
                    };
                    match client.secret_set(req).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!("✓ Secret '{key}' saved successfully for app '{app_name}'");
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                SecretCommands::List => {
                    match client.secret_list(&app_name).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                if data.keys.is_empty() {
                                    println!("App '{app_name}' has no secrets yet.");
                                } else {
                                    println!("Secret list for app '{app_name}':");
                                    for k in &data.keys {
                                        println!("  • {k}");
                                    }
                                }
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
            }
        }

        Commands::Promote { reason, version, detach } => {
            let app_name = match resolve_app(None) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };
            let req = PromoteRequest {
                app: app_name,
                reason,
                version,
            };

            match client.promote(req).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if let Some(data) = &resp.data {
                        if data.status == "pending_approval" {
                            let req_id = data.request_id.as_deref().unwrap_or("-");
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!("▸ Promotion submitted for admin approval (Request ID: {req_id})");
                            }
                            2
                        } else if let Some(dep) = &data.deployment {
                            if detach {
                                if json {
                                    output::print_json(&resp);
                                } else {
                                    println!("✓ Promotion started in background (detached).");
                                    println!("Deployment ID: {}", dep.deployment_id);
                                }
                                0
                            } else {
                                if !json {
                                    eprintln!("▸ Promoting {} → production", dep.app);
                                    eprintln!("  Version: {}  ID: {}", dep.version, dep.deployment_id);
                                }
                                stream_events_and_render(
                                    &client,
                                    &dep.deployment_id,
                                    &dep.app,
                                    "production",
                                    Some(&dep.version),
                                    json,
                                )
                                .await
                            }
                        } else {
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!("✓ Promotion processed successfully.");
                            }
                            0
                        }
                    } else {
                        1
                    }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Approvals => {
            match client.approvals().await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else if let Some(data) = &resp.data {
                        output::render_approvals(data);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Approve { request_id, note, detach } => {
            let req = ApproveRequest { request_id, note };
            match client.approve(req).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if let Some(data) = &resp.data {
                        if let Some(dep) = &data.deployment {
                            if detach {
                                if json {
                                    output::print_json(&resp);
                                } else {
                                    println!("✓ Approval request approved ({}). {}", data.action, data.detail);
                                    println!("Deployment ID: {}", dep.deployment_id);
                                }
                                0
                            } else {
                                if !json {
                                    println!("✓ Approval request approved ({}). {}", data.action, data.detail);
                                    eprintln!("▸ Running deployment {} → {}", dep.app, dep.env);
                                    eprintln!("  Version: {}  ID: {}", dep.version, dep.deployment_id);
                                }
                                stream_events_and_render(
                                    &client,
                                    &dep.deployment_id,
                                    &dep.app,
                                    &dep.env,
                                    Some(&dep.version),
                                    json,
                                )
                                .await
                            }
                        } else {
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!("✓ Approval request approved ({}): {}", data.action, data.detail);
                            }
                            0
                        }
                    } else {
                        1
                    }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Reject { request_id } => {
            let req = ApproveRequest { request_id, note: None };
            match client.reject(req).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else {
                        println!("✓ Approval request rejected successfully.");
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Rollback { env, to, detach } => {
            let app_name = match resolve_app(None) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };
            let req = RollbackRequest {
                app: app_name,
                env,
                target_version: to,
            };

            match client.rollback(req).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if let Some(data) = &resp.data {
                        if detach {
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!("✓ Rollback accepted in background (detached).");
                                println!("Deployment ID: {}", data.deployment_id);
                            }
                            0
                        } else {
                            if !json {
                                eprintln!("▸ Rollback {} → {}", data.app, data.env);
                                eprintln!("  Version: {}  ID: {}", data.version, data.deployment_id);
                            }
                            stream_events_and_render(
                                &client,
                                &data.deployment_id,
                                &data.app,
                                &data.env,
                                Some(&data.version),
                                json,
                            )
                            .await
                        }
                    } else {
                        1
                    }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Doctor => {
            match client.doctor().await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if json {
                        output::print_json(&resp);
                    } else if let Some(data) = &resp.data {
                        output::render_doctor(data);
                    }
                    let healthy = resp.data.map(|d| d.healthy).unwrap_or(false);
                    if healthy { 0 } else { 1 }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            }
        }

        Commands::Db { action } => {
            let app_name = match resolve_app(None) {
                Ok(a) => a,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            match action {
                DbCommands::Backup { service } => {
                    let req = DbBackupRequest { app: app_name, service };
                    match client.db_backup(req).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                let mb = (data.size_bytes as f64) / 1_048_576.0;
                                println!("✓ Database backup created successfully. ID: {} ({:.2} MB)", data.backup_id, mb);
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                DbCommands::List => {
                    match client.db_backups(&app_name).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                if data.backups.is_empty() {
                                    println!("No database backups yet for app '{app_name}'.");
                                } else {
                                    println!("{:<28} {:<12} {:<12} CREATED", "BACKUP ID", "SERVICE", "SIZE");
                                    println!("{}", "-".repeat(75));
                                    for b in &data.backups {
                                        let mb = (b.size_bytes as f64) / 1_048_576.0;
                                        let ts = b.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
                                        println!("{:<28} {:<12} {:<12.2} {}", b.backup_id, b.service, mb, ts);
                                    }
                                }
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                DbCommands::Restore { backup_id, service } => {
                    let req = DbRestoreRequest {
                        app: app_name,
                        service,
                        backup_id,
                    };
                    match client.db_restore(req).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                if data.status == "pending_approval" {
                                    let req_id = data.request_id.as_deref().unwrap_or("-");
                                    println!("▸ Database restore submitted for admin approval (Request ID: {req_id})");
                                    return 2;
                                } else {
                                    println!("✓ Database restored successfully.");
                                }
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
            }
        }

        Commands::App { action } => {
            match action {
                AppCommands::Delete { app } => {
                    let app_name = match resolve_app(app) {
                        Ok(a) => a,
                        Err(err) => {
                            output::print_error(&err, json);
                            return error_exit_code(&err);
                        }
                    };
                    match client.app_delete(&app_name).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                if data.status == "pending_approval" {
                                    let req_id = data.request_id.as_deref().unwrap_or("-");
                                    println!("▸ App deletion submitted for admin approval (Request ID: {req_id})");
                                    return 2;
                                } else {
                                    println!("✓ App '{app_name}' deleted successfully.");
                                }
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
            }
        }

        Commands::DefaultApp { app } => {
            if let Some(app_name) = app {
                let req = DefaultAppRequest { app: app_name.clone() };
                match client.default_app_put(req).await {
                    Ok(resp) => {
                        if !resp.success {
                            if let Some(err) = &resp.error {
                                output::print_error(err, json);
                                return error_exit_code(err);
                            }
                        }
                        if json {
                            output::print_json(&resp);
                        } else {
                            println!("✓ Default app set to: {app_name}");
                        }
                        0
                    }
                    Err(err) => {
                        output::print_error(&err, json);
                        error_exit_code(&err)
                    }
                }
            } else {
                match client.default_app_get().await {
                    Ok(resp) => {
                        if !resp.success {
                            if let Some(err) = &resp.error {
                                output::print_error(err, json);
                                return error_exit_code(err);
                            }
                        }
                        if json {
                            output::print_json(&resp);
                        } else if let Some(data) = &resp.data {
                            let curr = data.app.as_deref().unwrap_or("(none)");
                            println!("Current default app: {curr}");
                        }
                        0
                    }
                    Err(err) => {
                        output::print_error(&err, json);
                        error_exit_code(&err)
                    }
                }
            }
        }

        Commands::Token { action } => {
            match action {
                TokenCommands::Create { app, name } => {
                    let req = TokenCreateRequest { app, name };
                    match client.token_create(req).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                println!("✓ Token created successfully for '{}' ({}):", data.app, data.name);
                                println!("  Token: {}", data.token);
                                println!("  ID   : {}", data.id);
                                println!("  (IMPORTANT: Save this token now, it will not be displayed again)");
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                TokenCommands::List { app } => {
                    match client.token_list(&app).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                output::render_tokens(&data.tokens);
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                TokenCommands::Revoke { app, name } => {
                    match client.token_revoke(&app, &name).await {
                        Ok(resp) => {
                            if !resp.success {
                                if let Some(err) = &resp.error {
                                    output::print_error(err, json);
                                    return error_exit_code(err);
                                }
                            }
                            if json {
                                output::print_json(&resp);
                            } else if let Some(data) = &resp.data {
                                println!("✓ {} token(s) revoked successfully.", data.revoked);
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
            }
        }

        Commands::AgentGuide => {
            println!(
                r#"# Skoci Operational Guide for AI Agents

Skoci is a secure, deterministic agent-native deployment platform.

## Standard Workflow:
1. **Connect to Server**:
   `skoci connect --server <url> --token <token>`
   Configuration is stored in `.env.skoci` (permissions 0600) and added to `.gitignore`.
   NEVER print tokens to logs or commit them to public files.

2. **Initialize Project**:
   `skoci init`
   Skoci automatically detects the framework and generates `skoci.yaml`, `Dockerfile`, and `.dockerignore`.

3. **Deploy Application**:
   `skoci deploy --json`
   Use the `--json` flag to receive structured output (`ApiResponse<DeployResponse>`).
   On failure, inspect `error.fix` for an immediate actionable fix command.

4. **Exit Code Handling**:
   • Exit Code 0: Success.
   • Exit Code 1: Operational error (check error message & fix).
   • Exit Code 2: Action requires human approval. Wait for approval or prompt the user to run `skoci approvals` and `skoci approve <id>`.
   • Exit Code 3: Authentication error or quota rate limit (unauthorized, scope_violation, rate_limited).

5. **Agent Security Rules**:
   • NEVER use raw SSH if the task can be performed via Skoci CLI/API.
   • NEVER display secret values or tokens on console.
   • Use `skoci secret set <KEY> <VALUE>` to inject secure environment variables."#
            );
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_exit_code() {
        assert_eq!(error_exit_code(&SkociError::unauthorized("Unauthorized")), 3);
        assert_eq!(error_exit_code(&SkociError::scope_violation("a", "b")), 3);
        assert_eq!(error_exit_code(&SkociError::rate_limited()), 3);
        assert_eq!(error_exit_code(&SkociError::approval_required("req_1", "promote")), 2);
        assert_eq!(error_exit_code(&SkociError::bad_request("Bad", "Fix")), 1);
        assert_eq!(error_exit_code(&SkociError::internal("Err", "Fix")), 1);
    }

    #[test]
    fn test_resolve_app_explicit() {
        assert_eq!(resolve_app(Some("my-app".to_string())).unwrap(), "my-app");
        assert_eq!(resolve_app(Some("  custom-app  ".to_string())).unwrap(), "custom-app");
    }
}
