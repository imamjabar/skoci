use futures_util::StreamExt;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{
    ApiResponse, MAX_APP_NAME_LEN, PROJECT_CONFIG_PATH, PROJECT_DIR, PROJECT_DOCKERFILE_PATH,
    PROJECT_IGNORE_PATH,
};
use skoci_api::requests::{
    ApproveRequest, DbBackupRequest, DbRestoreRequest, DefaultAppRequest, DeployRequest,
    DeploymentsQuery, LogsQuery, PromoteRequest, RollbackRequest, SecretSetRequest,
    TokenCreateRequest,
};
use skoci_api::responses::{DeployResponse, DeployStage, InitResponse, StageStatus};
use skoci_client::SkociClient;
use skoci_core::config::{is_valid_app_name, ConfigLoader, RESERVED_APP_NAMES};
use skoci_core::detector::{normalize_app_name_lossy, ProjectDetector};
use std::io::IsTerminal;
use std::path::Path;
use std::time::Instant;

use crate::args::{
    AppCommands, Commands, DbCommands, DomainCommands, SecretCommands, TokenCommands,
};
use crate::output::{self, ValidateResponse};
use crate::progress::DeployProgressTracker;
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

    if let Some(config_path) = ConfigLoader::locate_config(path) {
        let cfg = ConfigLoader::load_config(&config_path)?;
        let trimmed = cfg.app.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        return Err(SkociError::config_invalid(
            format!("No 'app' name set in {}", config_path.display()),
            "Add a line such as 'app: my-app' to the configuration",
        ));
    }

    Err(SkociError::config_invalid(
        "No Skoci project configuration found here",
        "Run 'skoci init' to generate .skoci/skoci.yaml, or pass the app name as an argument",
    ))
}

pub fn resolve_app(provided: Option<String>) -> SkociResult<String> {
    resolve_app_in(Path::new("."), provided.as_deref())
}
#[derive(Debug, Default, Clone)]
pub struct GlobalOpts {
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub pin: Option<String>,
    pub profile: Option<String>,
}

pub fn resolve_global(global: &GlobalOpts) -> SkociResult<skoci_client::profile::ResolvedTarget> {
    let profiles = skoci_client::profile::load_profiles().unwrap_or_default();
    resolve_global_at(
        global,
        Path::new("/etc/skoci/skoci.env"),
        Path::new("."),
        &profiles,
    )
}

pub fn resolve_global_at(
    global: &GlobalOpts,
    master_env_path: &Path,
    project_root: &Path,
    profiles: &skoci_client::profile::ProfileConfig,
) -> SkociResult<skoci_client::profile::ResolvedTarget> {
    let project_env = crate::env::read_project_env_at(project_root);
    let mut target = skoci_client::profile::resolve_target_with_profiles_at(
        global.server_url.as_deref(),
        global.token.clone(),
        global.pin.as_deref(),
        global.profile.as_deref(),
        project_env,
        profiles,
        master_env_path,
    )?;

    if target.token.is_none() && crate::env::is_local_url(&target.url) {
        if let Some(master) = crate::env::local_master_token_from_path(master_env_path) {
            target.token = Some(master);
        }
    }

    Ok(target)
}

pub fn normalize_server_url(input: &str) -> Result<String, SkociError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(SkociError::bad_request(
            "Server URL cannot be empty",
            "Provide --server <url|ip|host>",
        ));
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }

    // Bare IPv4 literal
    if let Ok(ip4) = trimmed.parse::<std::net::Ipv4Addr>() {
        return Ok(format!(
            "https://{ip4}:{}",
            skoci_api::models::CONTROL_TLS_PORT
        ));
    }

    // Bare IPv6 literal
    if let Ok(ip6) = trimmed.parse::<std::net::Ipv6Addr>() {
        return Ok(format!(
            "https://[{ip6}]:{}",
            skoci_api::models::CONTROL_TLS_PORT
        ));
    }

    // Bracketed IPv6 literal with port, e.g. [::1]:8443
    if trimmed.starts_with('[') {
        return Ok(format!("https://{trimmed}"));
    }

    // Host or host:port
    Ok(format!("https://{trimmed}"))
}

pub fn parse_semver_triplet(ver: &str) -> Option<(u64, u64, u64)> {
    let s = ver.trim().trim_start_matches(['v', 'V']);
    let mut parts = s.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_part = parts.next()?;
    let patch_str = patch_part.split(['-', '+']).next()?;
    let patch = patch_str.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

pub fn check_version_compatibility(cli: &str, server: &str) -> Result<Option<String>, SkociError> {
    let (c_maj, c_min, c_patch) = match parse_semver_triplet(cli) {
        Some(t) => t,
        None => return Err(SkociError::version_mismatch(cli, server)),
    };
    let (s_maj, s_min, s_patch) = match parse_semver_triplet(server) {
        Some(t) => t,
        None => return Err(SkociError::version_mismatch(cli, server)),
    };

    if c_maj != s_maj {
        return Err(SkociError::version_mismatch(cli, server));
    }

    if c_maj == 0 {
        if c_min != s_min {
            return Err(SkociError::version_mismatch(cli, server));
        }
        if c_patch != s_patch {
            return Ok(Some(format!(
                "Warning: CLI version ({cli}) differs from server version ({server}) by patch version."
            )));
        }
    } else {
        if c_min != s_min {
            return Ok(Some(format!(
                "Warning: CLI minor version ({cli}) differs from server version ({server})."
            )));
        }
        if c_patch != s_patch {
            return Ok(Some(format!(
                "Warning: CLI version ({cli}) differs from server version ({server}) by patch version."
            )));
        }
    }

    Ok(None)
}

async fn verify_version_compatibility(client: &SkociClient, json: bool) -> Result<(), i32> {
    match client.doctor().await {
        Ok(resp) => {
            if let Some(doc) = &resp.data {
                match check_version_compatibility(env!("CARGO_PKG_VERSION"), &doc.version) {
                    Ok(Some(warning)) => {
                        if !json {
                            eprintln!("{warning}");
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        output::print_error(&err, json);
                        return Err(error_exit_code(&err));
                    }
                }
            }
            Ok(())
        }
        Err(err) => {
            output::print_error(&err, json);
            Err(error_exit_code(&err))
        }
    }
}

fn validate_app_name_for_init(app_name: &str) -> SkociResult<()> {
    if is_valid_app_name(app_name) {
        return Ok(());
    }

    let rule_broken = if app_name.is_empty() {
        "app name cannot be empty"
    } else if app_name.len() > MAX_APP_NAME_LEN {
        "app name exceeds 40-character limit"
    } else if RESERVED_APP_NAMES.contains(&app_name) {
        "app name is reserved (skoci, api, www, admin, dashboard)"
    } else if app_name.ends_with("-preview") {
        "app name cannot end with '-preview' suffix"
    } else if app_name.contains("--") {
        "app name cannot contain consecutive hyphens ('--')"
    } else if app_name.starts_with('-') || app_name.ends_with('-') {
        "app name cannot start or end with a hyphen"
    } else if !app_name
        .chars()
        .next()
        .map(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .unwrap_or(false)
    {
        "app name must start with a lowercase letter or digit"
    } else {
        "app name contains invalid characters (only lowercase alphanumeric and hyphens allowed)"
    };

    let fix = format!(
        "Pass a valid app name with 'skoci init --app <name>'. Rule broken: {rule_broken} (must be 1-40 chars of [a-z0-9-], starting with a letter or digit, no '--', no trailing '-', no '-preview' suffix, and not reserved)"
    );

    Err(SkociError::config_invalid(
        format!("Invalid app name '{app_name}': {rule_broken}"),
        fix,
    ))
}

fn read_existing_app_name(path: &Path) -> Option<String> {
    let cfg_path = ConfigLoader::locate_config(path)?;
    let content = std::fs::read_to_string(cfg_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("app:") {
            let val = rest.trim().trim_matches(['"', '\'']).trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

pub fn init_project(
    path: impl AsRef<Path>,
    app_override: Option<&str>,
) -> SkociResult<InitResponse> {
    let path = path.as_ref();
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| {
            SkociError::internal(
                format!("Failed to create directory {}: {e}", path.display()),
                "Ensure the directory can be created and permissions are sufficient",
            )
        })?;
    }

    crate::env::migrate_legacy_project_layout(path)?;

    let app_name = if let Some(app) = app_override {
        app.trim().to_string()
    } else if let Some(existing) = read_existing_app_name(path) {
        existing
    } else if let Some(suggested) = ProjectDetector::suggest_app_name(path) {
        suggested
    } else {
        path.canonicalize()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| normalize_app_name_lossy(&n.to_string_lossy()))
            })
            .or_else(|| {
                path.file_name()
                    .map(|n| normalize_app_name_lossy(&n.to_string_lossy()))
            })
            .unwrap_or_default()
    };

    validate_app_name_for_init(&app_name)?;

    let detection = ProjectDetector::detect(path);

    let skoci_dir = path.join(PROJECT_DIR);
    if !skoci_dir.exists() {
        std::fs::create_dir_all(&skoci_dir).map_err(|e| {
            SkociError::internal(
                format!("Failed to create directory {}: {e}", skoci_dir.display()),
                "Ensure the directory can be created and permissions are sufficient",
            )
        })?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&skoci_dir, std::fs::Permissions::from_mode(0o700));
    }

    let mut created_files = Vec::new();
    let mut skipped_files = Vec::new();

    let yaml_file = path.join(PROJECT_CONFIG_PATH);
    if !yaml_file.exists() {
        let content = ProjectDetector::generate_skoci_yaml(&app_name, &detection);
        std::fs::write(&yaml_file, content).map_err(|e| {
            SkociError::internal(
                format!("Failed to write {}: {e}", PROJECT_CONFIG_PATH),
                "Check directory write permissions",
            )
        })?;
        created_files.push(PROJECT_CONFIG_PATH.to_string());
    } else {
        skipped_files.push(PROJECT_CONFIG_PATH.to_string());
    }

    if let Some(df_content) = ProjectDetector::dockerfile(&detection) {
        let df_file = path.join(PROJECT_DOCKERFILE_PATH);
        if !df_file.exists() {
            std::fs::write(&df_file, df_content).map_err(|e| {
                SkociError::internal(
                    format!("Failed to write {}: {e}", PROJECT_DOCKERFILE_PATH),
                    "Check directory write permissions",
                )
            })?;
            created_files.push(PROJECT_DOCKERFILE_PATH.to_string());
        } else {
            skipped_files.push(PROJECT_DOCKERFILE_PATH.to_string());
        }
    }

    let ignore_file = path.join(PROJECT_IGNORE_PATH);
    if !ignore_file.exists() {
        let ignore_content = ProjectDetector::upload_ignore();
        std::fs::write(&ignore_file, ignore_content).map_err(|e| {
            SkociError::internal(
                format!("Failed to write {}: {e}", PROJECT_IGNORE_PATH),
                "Check directory write permissions",
            )
        })?;
        created_files.push(PROJECT_IGNORE_PATH.to_string());
    } else {
        skipped_files.push(PROJECT_IGNORE_PATH.to_string());
    }

    crate::env::ensure_gitignore_ignores_env(path)?;

    Ok(InitResponse {
        detected_framework: detection.framework.name().to_string(),
        created_files,
        app_name,
        skipped_files,
        notes: detection.notes,
    })
}

pub fn validate_project(path: impl AsRef<Path>) -> SkociResult<ValidateResponse> {
    validate_project_with_options(path, false)
}

pub fn validate_project_with_options(
    path: impl AsRef<Path>,
    verbose: bool,
) -> SkociResult<ValidateResponse> {
    let path = path.as_ref();
    crate::env::migrate_legacy_project_layout(path)?;
    let config_path = ConfigLoader::locate_config(path).ok_or_else(|| {
        SkociError::config_invalid(
            "skoci.yaml file not found",
            "Run 'skoci init' in your project root or pass a valid directory",
        )
    })?;

    let cfg = ConfigLoader::load_config(&config_path)?;
    let (files, bytes, excluded_sensitive) = if verbose {
        let df = if cfg.build.dockerfile.is_empty() {
            PROJECT_DOCKERFILE_PATH
        } else {
            cfg.build.dockerfile.as_str()
        };
        let listing = skoci_client::package::list_context_files(path, df)?;
        (
            Some(listing.files),
            Some(listing.bytes),
            Some(listing.excluded_sensitive),
        )
    } else {
        (None, None, None)
    };

    Ok(ValidateResponse {
        config_path: config_path.display().to_string(),
        app: cfg.app,
        domains: cfg.domains,
        preview_domains: cfg.preview_domains,
        dockerfile: cfg.build.dockerfile,
        context: cfg.build.context,
        required_secrets: cfg.env.secrets,
        files,
        bytes,
        excluded_sensitive,
    })
}

pub struct ConnectCredentials<'a> {
    pub name: &'a str,
    pub server_url: &'a str,
    pub token: &'a str,
    pub pin: Option<&'a str>,
    pub project_env: bool,
    pub no_profile: bool,
}

pub fn persist_connect_credentials_at(
    project_root: &Path,
    profile_path: &Path,
    creds: &ConnectCredentials<'_>,
) -> SkociResult<Vec<String>> {
    let mut stores = Vec::new();

    if creds.project_env {
        crate::env::write_project_env_at(project_root, creds.server_url, creds.token, creds.pin)?;
        crate::env::ensure_gitignore_ignores_env(project_root)?;
        stores.push(".skoci/env (mode 0600)".to_string());
    }

    if !creds.no_profile {
        let mut config = skoci_client::profile::load_profiles_from(profile_path)?;
        config.upsert(creds.name, creds.server_url, creds.token, creds.pin)?;
        config.current = Some(creds.name.to_string());
        skoci_client::profile::save_profiles_to(profile_path, &config)?;
        stores.push(format!(
            "~/.config/skoci/servers.json (profile '{}', mode 0600)",
            creds.name
        ));
    }

    Ok(stores)
}

pub fn agent_guide_text() -> &'static str {
    r#"# Skoci Operational Guide for AI Agents

Skoci is a secure, deterministic agent-native deployment platform.

## Standard Workflow:
1. **Bootstrap or Connect to Server**:
   • Bootstrap a fresh VPS over SSH (installs Skoci, mints a scoped token, and saves the pinned profile):
     `skoci server setup --host <ip> --user <user>`
   • Connect to an existing server (bare-IP HTTPS requires the server SPKI pin):
     `skoci connect --server https://<ip>:8443 --token <token> --pin sha256:<64 hex>`
   Credentials live in `~/.config/skoci/servers.json` (mode 0600); pass `--project-env` only when CI or per-repo `.skoci/env` (0600) is needed.
   NEVER print tokens to logs or commit them to public files.

2. **Initialize Project**:
   `skoci init`
   Skoci automatically detects the framework and generates `.skoci/skoci.yaml`, `.skoci/Dockerfile`, and `.skoci/ignore`.
   Use `skoci validate --verbose` to inspect the exact file list that `skoci deploy` will upload.

3. **Deploy Application**:
   `skoci deploy --json`
   Use the `--json` flag to receive structured output (`ApiResponse<DeployResponse>`).
   On failure, inspect `error.fix` for an immediate actionable fix command.

4. **Exit Code Handling**:
   • Exit Code 0: Success.
   • Exit Code 1: Operational error (check error message & fix).
   • Exit Code 2: Action requires human approval. Wait for approval or prompt the user to run `sudo skoci approvals` and `sudo skoci approve <id>` on the server.
   • Exit Code 3: Authentication error or quota rate limit (unauthorized, scope_violation, rate_limited).

5. **Promotion & Approval Flow (Exit Code 2)**:
   • Master token promotes directly without approval.
   • For a scoped agent token (`sk_agent_*`), an app's FIRST promotion to production (when no production route exists yet) succeeds immediately without creating an approval request.
   • Every later promotion to production by a scoped agent token produces an approval request with status `pending_approval` (`appr_…`) and exits with code 2.
   • A server started with `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` (configured via `skoci server setup --strict-approvals`) gates the first promotion too.
   • The agent CANNOT approve its own request. There is no auto-approve flag.
   • When `skoci promote` exits 2 with `pending_approval`, extract and surface `Request ID: appr_…` to the human operator.
   • The human operator must approve the request on the server by running `sudo skoci approve <id>` (server-side admin commands require `sudo` because `/etc/skoci/skoci.env` is `0600 root:root`).
   • `skoci approvals --json` lists pending requests under `data[]`.
   • Note: MCP intentionally exposes only the listing tool (`skoci_approvals`), not approve or reject tools.

6. **Agent Security Rules**:
   • NEVER use raw SSH if the task can be performed via Skoci CLI/API.
   • Server-side admin commands (`skoci approve`, `skoci reject`, `skoci token create`, etc. when using local master credentials on the VPS) require `sudo` because `/etc/skoci/skoci.env` is `0600 root:root`.
   • NEVER display secret values or tokens on console.
   • Use `skoci secret set <KEY> <VALUE>` to inject secure environment variables."#
}

pub async fn stream_events_and_render(
    client: &SkociClient,
    deployment_id: &str,
    _app: &str,
    _env: &str,
    version: Option<&str>,
    estimated_secs: Option<u64>,
    is_json: bool,
) -> i32 {
    let mut stream = client.follow_events(deployment_id);
    let is_term = std::io::stderr().is_terminal();
    let start_time = Instant::now();

    if !is_json && version.is_none() {
        eprintln!("▸ Watching deployment: {}", deployment_id);
    }

    let mut accumulated_logs = Vec::new();
    let mut tracker = DeployProgressTracker::new(estimated_secs);
    let mut rendered_lines: usize = 0;
    let mut final_response: Option<DeployResponse> = None;
    let mut final_error: Option<SkociError> = None;

    if is_term && !is_json {
        use std::io::Write;
        let mut out = std::io::stderr().lock();
        let lines = tracker.render_full(true);
        for l in &lines {
            let _ = writeln!(out, "{l}");
        }
        let _ = out.flush();
        rendered_lines = lines.len();
    }

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

                tracker.update_progress(event.progress);

                match event.status {
                    StageStatus::Log => {
                        accumulated_logs.push(event.message.clone());
                        if accumulated_logs.len() > 100 {
                            accumulated_logs.remove(0);
                        }
                        tracker.on_log(event.message);
                    }
                    StageStatus::Running => {
                        let step = tracker.on_stage_running(event.stage);
                        if !is_term {
                            if let Some(n) = step {
                                eprintln!("▸ [{n}/8] {}", event.stage.label());
                            }
                        }
                    }
                    StageStatus::Ok => {
                        let step_info = tracker.on_stage_ok(event.stage);
                        if !is_term {
                            if let Some((n, dur)) = step_info {
                                eprintln!(
                                    "✓ [{n}/8] {} ({:.1}s)",
                                    event.stage.label(),
                                    dur.as_secs_f64()
                                );
                            }
                        }
                    }
                    StageStatus::Skipped => {
                        let step = tracker.on_stage_skipped(event.stage);
                        if !is_term {
                            if let Some(n) = step {
                                eprintln!("– [{n}/8] {} (skipped)", event.stage.label());
                            }
                        }
                    }
                    StageStatus::Failed => {
                        let step = tracker.on_stage_failed(event.stage, event.message.clone());
                        if !is_term {
                            let detail = if event.message.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", event.message)
                            };
                            if let Some(n) = step {
                                eprintln!("✗ [{n}/8] {}{detail}", event.stage.label());
                            }
                        }
                    }
                }

                if is_term {
                    use std::io::Write;
                    let mut out = std::io::stderr().lock();
                    for _ in 0..rendered_lines {
                        let _ = write!(out, "\x1b[1A\x1b[2K\r");
                    }
                    let lines = tracker.render_full(true);
                    for l in &lines {
                        let _ = writeln!(out, "{l}");
                    }
                    let _ = out.flush();
                    rendered_lines = lines.len();
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
        if resp.urls.is_empty() {
            if is_term {
                eprintln!(
                    "\x1b[1;33m⚠ No public address:\x1b[0m container is running but unreachable"
                );
            } else {
                eprintln!("⚠ No public address: container is running but unreachable");
            }
            eprintln!(
                "  Add a domain you own to .skoci/skoci.yaml, or run: skoci default-app {}",
                resp.app
            );
        }
        if resp.status == "degraded" {
            if let Some(reverted) = &resp.evidence.reverted_to_version {
                if is_term {
                    eprintln!(
                        "\x1b[1;33m⚠ Rolled back:\x1b[0m version {} failed its smoke test; {reverted} is serving again",
                        resp.version
                    );
                } else {
                    eprintln!(
                        "⚠ Rolled back: version {} failed its smoke test; {reverted} is serving again",
                        resp.version
                    );
                }
            } else {
                for url in &resp.urls {
                    if is_term {
                        eprintln!("\x1b[1;33m⚠ Live but failing:\x1b[0m {url}");
                    } else {
                        eprintln!("⚠ Live but failing: {url}");
                    }
                }
            }
            if let Some(code) = resp.evidence.smoke_test_status {
                eprintln!("Smoke test returned HTTP {code} (expected 2xx/3xx/401)");
            } else {
                eprintln!("Smoke test did not pass");
            }
            eprintln!("Version: {}  ID: {}  Duration: {:.1}s", resp.version, resp.deployment_id, dur);
            eprintln!("\nHints:");
            eprintln!("  skoci logs {}", resp.app);
            eprintln!("  skoci status {}", resp.app);
            1
        } else if resp.status == "failed" {
            if is_term {
                eprintln!("\n\x1b[1;31m✗ Deployment failed\x1b[0m");
            } else {
                eprintln!("\n✗ Deployment failed");
            }
            1
        } else {
            for url in &resp.urls {
                let pending = resp.unready_urls.iter().any(|u| u == url);
                match (is_term, pending) {
                    (true, false) => println!("\x1b[1;32m✓ Live:\x1b[0m {url}"),
                    (true, true) => {
                        println!("\x1b[1;33m◷ Not serving yet:\x1b[0m {url}")
                    }
                    (false, false) => println!("✓ Live: {url}"),
                    (false, true) => println!("◷ Not serving yet: {url}"),
                }
            }
            if !resp.unready_urls.is_empty() {
                println!(
                    "  The container is healthy; the address above did not answer through the proxy yet."
                );
                println!("  A new certificate usually needs under a minute. Re-check with: skoci status {}", resp.app);
            }
            println!("Version: {}  ID: {}  Duration: {:.1}s", resp.version, resp.deployment_id, dur);
            println!("\nHints:");
            println!("  skoci logs {}", resp.app);
            println!("  skoci status {}", resp.app);
            0
        }
    } else if let Some(err) = final_error {
        if !accumulated_logs.is_empty() {
            eprintln!("\nLast log summary:");
            for line in accumulated_logs.iter().rev().take(30).rev() {
                eprintln!("  │ {line}");
            }
        }
        if is_term {
            eprintln!("\n\x1b[1;31m✗ Deployment failed:\x1b[0m {}", err.message);
        } else {
            eprintln!("\n✗ Deployment failed: {}", err.message);
        }
        eprintln!("Fix: {}", err.fix);
        if let Some(v) = version {
            eprintln!("Version: {v}");
        }
        eprintln!("Deployment ID: {deployment_id}  (skoci watch {deployment_id})");
        error_exit_code(&err)
    } else {
        eprintln!("\n✗ Stream completed without final deployment status");
        eprintln!("Fix: Use `skoci watch {}` to reconnect", deployment_id);
        1
    }
}

pub async fn run_command(cmd: Commands, global: GlobalOpts, json: bool) -> i32 {
    if json {
        skoci_client::profile::suppress_desync_warning();
    }
    if let Commands::Deploy { ref path, .. } = cmd {
        for dir in [Path::new("."), path.as_path()] {
            if let Err(err) = crate::env::migrate_legacy_project_layout(dir) {
                output::print_error(&err, json);
                return error_exit_code(&err);
            }
        }
    }
    let client = if !matches!(
        cmd,
        Commands::Connect { .. }
            | Commands::Init { .. }
            | Commands::AgentGuide
            | Commands::Validate { .. }
    ) {
        let target = match resolve_global(&global) {
            Ok(t) => t,
            Err(err) => {
                output::print_error(&err, json);
                return error_exit_code(&err);
            }
        };
        match SkociClient::new(&target.url, target.token, target.pin) {
            Ok(c) => c,
            Err(err) => {
                output::print_error(&err, json);
                return error_exit_code(&err);
            }
        }
    } else {
        match SkociClient::new("http://127.0.0.1:8787", None, None) {
            Ok(c) => c,
            Err(_) => return 1,
        }
    };

    match cmd {
        Commands::Connect {
            server,
            token: conn_token,
            insecure,
            pin,
            name,
            no_profile,
            project_env,
        } => {
            if let Err(err) = crate::env::migrate_legacy_project_layout(Path::new(".")) {
                output::print_error(&err, json);
                return error_exit_code(&err);
            }
            let normalized_server = match normalize_server_url(&server) {
                Ok(s) => s,
                Err(e) => {
                    output::print_error(&e, json);
                    return 1;
                }
            };

            if normalized_server.to_lowercase().starts_with("http://")
                && !crate::env::is_local_url(&normalized_server)
                && !insecure
            {
                let err = SkociError::bad_request(
                    format!(
                        "Server URL uses unencrypted HTTP protocol ('{normalized_server}'). Token will be sent in plaintext."
                    ),
                    "Use HTTPS ('https://...'), an SSH tunnel (localhost), or add the --insecure flag if you understand the risks.",
                );
                output::print_error(&err, json);
                return 1;
            }

            let resolved_token = match conn_token {
                Some(t) if !t.trim().is_empty() => t.trim().to_string(),
                _ => match std::env::var("SKOCI_TOKEN") {
                    Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
                    _ => {
                        use std::io::{BufRead, IsTerminal, Write};
                        if std::io::stdin().is_terminal() {
                            eprint!("Enter token: ");
                            let _ = std::io::stderr().flush();
                        }
                        let mut line = String::new();
                        let _ = std::io::stdin().lock().read_line(&mut line);
                        let trimmed = line.trim().to_string();
                        if trimmed.is_empty() {
                            let err = SkociError::bad_request(
                                "Token cannot be empty",
                                "Provide --token <token>, set SKOCI_TOKEN, or enter via stdin",
                            );
                            output::print_error(&err, json);
                            return 1;
                        }
                        trimmed
                    }
                },
            };

            let test_client = match SkociClient::new(
                &normalized_server,
                Some(resolved_token.clone()),
                pin.clone(),
            ) {
                Ok(c) => c,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            let doc_resp = match test_client.doctor().await {
                Ok(resp) => resp,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };

            if !doc_resp.success {
                if let Some(err) = &doc_resp.error {
                    output::print_error(err, json);
                    return error_exit_code(err);
                }
            }

            if let Some(doc) = &doc_resp.data {
                match check_version_compatibility(env!("CARGO_PKG_VERSION"), &doc.version) {
                    Ok(Some(warning)) => {
                        if !json {
                            eprintln!("{warning}");
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        output::print_error(&err, json);
                        return error_exit_code(&err);
                    }
                }
            }

            let stores = match persist_connect_credentials_at(
                Path::new("."),
                &skoci_client::profile::default_profile_path(),
                &ConnectCredentials {
                    name: &name,
                    server_url: &normalized_server,
                    token: &resolved_token,
                    pin: pin.as_deref(),
                    project_env,
                    no_profile,
                },
            ) {
                Ok(s) => s,
                Err(err) => {
                    output::print_error(&err, json);
                    return 1;
                }
            };

            if json {
                output::print_json(&doc_resp);
            } else {
                if stores.is_empty() {
                    println!("✓ Successfully connected (no credential store written)\n");
                } else {
                    println!(
                        "✓ Successfully connected and saved configuration to {}\n",
                        stores.join(" and ")
                    );
                }
                if let Some(doc) = &doc_resp.data {
                    output::render_doctor(doc);
                }
            }
            0
        }

        Commands::Init { app, path } => match init_project(&path, app.as_deref()) {
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

                    let config_file = PROJECT_CONFIG_PATH;

                    let resolved_server_url = (|| {
                        let project_env = crate::env::read_project_env_at(&path);
                        if let Ok(profiles) = skoci_client::profile::load_profiles() {
                            if let Ok(target) = skoci_client::profile::resolve_target_with_profiles(
                                None,
                                None,
                                None,
                                None,
                                project_env.clone(),
                                &profiles,
                            ) {
                                if matches!(
                                    target.source,
                                    skoci_client::profile::TargetSource::ProjectEnv
                                        | skoci_client::profile::TargetSource::Profile(_)
                                ) {
                                    return Some(target.url);
                                }
                            }
                        }
                        project_env.map(|(url, _, _)| url)
                    })();

                    println!("\nNext steps:");
                    println!("  1. Review {config_file} (port, health path, domains)");
                    if let Some(target_url) = resolved_server_url {
                        println!("  2. Deploy to {target_url}: skoci deploy");
                    } else {
                        println!("  2. Connect once: skoci connect --server <url> --token <token>");
                        println!("  3. Deploy: skoci deploy");
                    }
                }
                0
            }
            Err(err) => {
                output::print_error(&err, json);
                error_exit_code(&err)
            }
        },

        Commands::Validate { path, verbose } => {
            match validate_project_with_options(&path, verbose) {
                Ok(res) => {
                    if json {
                        output::print_json(&ApiResponse::ok(res, "Configuration is valid"));
                    } else {
                        output::render_validate(&res);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    1
                }
            }
        }

        Commands::Deploy {
            env,
            path,
            detach,
            commit,
        } => {
            if let Err(err) = crate::env::migrate_legacy_project_layout(&path) {
                output::print_error(&err, json);
                return error_exit_code(&err);
            }
            if let Err(code) = verify_version_compatibility(&client, json).await {
                return code;
            }
            let config_path = ConfigLoader::locate_config(&path).ok_or_else(|| {
                SkociError::config_invalid(
                    "skoci.yaml file not found",
                    "Run 'skoci init' in your project root or pass --path <dir>",
                )
            });
            let config_path = match config_path {
                Ok(p) => p,
                Err(err) => {
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }
            };
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
                    let mut cmd = tokio::process::Command::new("git");
                    cmd.args(["rev-parse", "--short", "HEAD"])
                        .current_dir(&path)
                        .env_clear();
                    if let Ok(p) = std::env::var("PATH") {
                        cmd.env("PATH", p);
                    }
                    if let Ok(h) = std::env::var("HOME") {
                        cmd.env("HOME", h);
                    }
                    let out = cmd.output().await;
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

            let resp = client
                .deploy_upload(&packed.path, &req, move |_sent, _total| {})
                .await;

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
                            let err = SkociError::internal(
                                "Empty response from server",
                                "Try deploying again",
                            );
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
                            println!(
                                "Use `skoci watch {}` to monitor status.",
                                accepted.deployment_id
                            );
                        }
                        0
                    } else {
                        stream_events_and_render(
                            &client,
                            &accepted.deployment_id,
                            &app,
                            &env,
                            Some(&accepted.version),
                            accepted.estimated_secs,
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
            stream_events_and_render(&client, &deployment_id, "app", "env", None, None, json).await
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
                    let resolved_value = match value {
                        Some(v) if !v.is_empty() => v,
                        _ => {
                            use std::io::{BufRead, IsTerminal, Write};
                            if std::io::stdin().is_terminal() {
                                eprint!("Enter secret value: ");
                                let _ = std::io::stderr().flush();
                            }
                            let mut line = String::new();
                            let _ = std::io::stdin().lock().read_line(&mut line);
                            let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                            if trimmed.is_empty() {
                                let err = SkociError::bad_request(
                                    "Secret value cannot be empty",
                                    "Provide value as argument or via stdin: echo -n 'val' | skoci secret set KEY",
                                );
                                output::print_error(&err, json);
                                return 1;
                            }
                            trimmed
                        }
                    };
                    let req = SecretSetRequest {
                        app: app_name.clone(),
                        key: key.clone(),
                        value: resolved_value,
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
                                println!(
                                    "✓ Secret '{key}' saved successfully for app '{app_name}'"
                                );
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                SecretCommands::List => match client.secret_list(&app_name).await {
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
                },
            }
        }

        Commands::Promote {
            reason,
            version,
            detach,
        } => {
            if let Err(code) = verify_version_compatibility(&client, json).await {
                return code;
            }
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
                                    eprintln!(
                                        "  Version: {}  ID: {}",
                                        dep.version, dep.deployment_id
                                    );
                                }
                                stream_events_and_render(
                                    &client,
                                    &dep.deployment_id,
                                    &dep.app,
                                    "production",
                                    Some(&dep.version),
                                    dep.estimated_secs,
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

        Commands::Approvals => match client.approvals().await {
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
        },

        Commands::Approve {
            request_id,
            note,
            detach,
        } => {
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
                                    println!(
                                        "✓ Approval request approved ({}). {}",
                                        data.action, data.detail
                                    );
                                    println!("Deployment ID: {}", dep.deployment_id);
                                }
                                0
                            } else {
                                if !json {
                                    println!(
                                        "✓ Approval request approved ({}). {}",
                                        data.action, data.detail
                                    );
                                    eprintln!("▸ Running deployment {} → {}", dep.app, dep.env);
                                    eprintln!(
                                        "  Version: {}  ID: {}",
                                        dep.version, dep.deployment_id
                                    );
                                }
                                stream_events_and_render(
                                    &client,
                                    &dep.deployment_id,
                                    &dep.app,
                                    &dep.env,
                                    Some(&dep.version),
                                    dep.estimated_secs,
                                    json,
                                )
                                .await
                            }
                        } else {
                            if json {
                                output::print_json(&resp);
                            } else {
                                println!(
                                    "✓ Approval request approved ({}): {}",
                                    data.action, data.detail
                                );
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
            let req = ApproveRequest {
                request_id,
                note: None,
            };
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
            if let Err(code) = verify_version_compatibility(&client, json).await {
                return code;
            }
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
                                eprintln!(
                                    "  Version: {}  ID: {}",
                                    data.version, data.deployment_id
                                );
                            }
                            stream_events_and_render(
                                &client,
                                &data.deployment_id,
                                &data.app,
                                &data.env,
                                Some(&data.version),
                                data.estimated_secs,
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

        Commands::Doctor => match client.doctor().await {
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
                if healthy {
                    0
                } else {
                    1
                }
            }
            Err(err) => {
                output::print_error(&err, json);
                error_exit_code(&err)
            }
        },

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
                    let req = DbBackupRequest {
                        app: app_name,
                        service,
                    };
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
                                println!(
                                    "✓ Database backup created successfully. ID: {} ({:.2} MB)",
                                    data.backup_id, mb
                                );
                            }
                            0
                        }
                        Err(err) => {
                            output::print_error(&err, json);
                            error_exit_code(&err)
                        }
                    }
                }
                DbCommands::List => match client.db_backups(&app_name).await {
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
                                println!(
                                    "{:<28} {:<12} {:<12} CREATED",
                                    "BACKUP ID", "SERVICE", "SIZE"
                                );
                                println!("{}", "-".repeat(75));
                                for b in &data.backups {
                                    let mb = (b.size_bytes as f64) / 1_048_576.0;
                                    let ts = b.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
                                    println!(
                                        "{:<28} {:<12} {:<12.2} {}",
                                        b.backup_id, b.service, mb, ts
                                    );
                                }
                            }
                        }
                        0
                    }
                    Err(err) => {
                        output::print_error(&err, json);
                        error_exit_code(&err)
                    }
                },
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

        Commands::App { action } => match action {
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
        },

        Commands::DefaultApp { app } => {
            if let Some(app_name) = app {
                let req = DefaultAppRequest {
                    app: app_name.clone(),
                };
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

        Commands::Token { action } => match action {
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
                            println!(
                                "✓ Token created successfully for '{}' ({}):",
                                data.app, data.name
                            );
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
            TokenCommands::List { app } => match client.token_list(&app).await {
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
            },
            TokenCommands::Revoke { app, name } => match client.token_revoke(&app, &name).await {
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
            },
        },
        Commands::Domain { cmd } => match cmd {
            DomainCommands::Check { host } => match client.domain_check(&host).await {
                Ok(resp) => {
                    if json {
                        output::print_json(&resp);
                    } else if let Some(check) = &resp.data {
                        output::render_domain_check(check);
                    } else if let Some(err) = &resp.error {
                        output::print_error(err, json);
                        return error_exit_code(err);
                    }
                    0
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            },
            DomainCommands::Add { host } => match client.domain_check(&host).await {
                Ok(resp) => {
                    if !resp.success {
                        if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        }
                    }
                    if let Some(check) = &resp.data {
                        if !check.points_here {
                            let err = SkociError::dns_mismatch(
                                &check.host,
                                &check.expected_ips,
                                &check.resolved_ips,
                            );
                            output::print_error(&err, json);
                            return error_exit_code(&err);
                        }
                        if json {
                            output::print_json(&resp);
                        } else {
                            println!(
                                "✓ Domain '{host}' is verified and correctly points to this server.\n"
                            );
                            println!("Add the domain to your `skoci.yaml` under `domains:`:\n");
                            println!("domains:");
                            println!("  - {host}\n");
                        }
                        0
                    } else if let Some(err) = &resp.error {
                        output::print_error(err, json);
                        error_exit_code(err)
                    } else {
                        let err = SkociError::internal(
                            "Missing domain check response data",
                            "Retry the check",
                        );
                        output::print_error(&err, json);
                        error_exit_code(&err)
                    }
                }
                Err(err) => {
                    output::print_error(&err, json);
                    error_exit_code(&err)
                }
            },
            DomainCommands::Set {
                host,
                primary,
                force,
            } => {
                if !primary {
                    let err = SkociError::bad_request(
                        "Missing required flag '--primary': non-primary domain configuration is not supported yet",
                        format!("skoci domain set {host} --primary"),
                    );
                    output::print_error(&err, json);
                    return error_exit_code(&err);
                }

                match client.domain_set_primary(&host, force).await {
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
                            println!("✓ Primary base domain updated successfully\n");
                            match &data.previous_base_domain {
                                Some(prev) => {
                                    println!("Base domain     : {prev} → {}", data.base_domain)
                                }
                                None => println!("Base domain     : {}", data.base_domain),
                            }
                            println!("Control hostname: {}", data.api_domain);

                            if !data.certificate_hosts.is_empty() {
                                println!("\nCertificate hostnames to issue:");
                                for h in &data.certificate_hosts {
                                    println!("  • {h}");
                                }
                                println!("  ⚠ Certificate issuance takes a few seconds; the first request to a new hostname may be slow.");
                            }

                            if !data.updated_routes.is_empty() {
                                println!("\nRe-pointed routes:");
                                for route in &data.updated_routes {
                                    println!("  • {route}");
                                }
                            }

                            if data.dns_check_skipped {
                                println!("\n⚠ DNS pre-check skipped (--force).");
                            }
                        } else if let Some(err) = &resp.error {
                            output::print_error(err, json);
                            return error_exit_code(err);
                        } else {
                            let err = SkociError::internal(
                                "Missing domain set response data",
                                "Retry setting primary domain",
                            );
                            output::print_error(&err, json);
                            return error_exit_code(&err);
                        }
                        0
                    }
                    Err(err) => {
                        output::print_error(&err, json);
                        error_exit_code(&err)
                    }
                }
            }
        },

        Commands::AgentGuide => {
            println!("{}", agent_guide_text());
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_exit_code() {
        assert_eq!(
            error_exit_code(&SkociError::unauthorized("Unauthorized")),
            3
        );
        assert_eq!(error_exit_code(&SkociError::scope_violation("a", "b")), 3);
        assert_eq!(error_exit_code(&SkociError::rate_limited()), 3);
        assert_eq!(
            error_exit_code(&SkociError::approval_required("req_1", "promote")),
            2
        );
        assert_eq!(error_exit_code(&SkociError::bad_request("Bad", "Fix")), 1);
        assert_eq!(error_exit_code(&SkociError::internal("Err", "Fix")), 1);
    }

    #[test]
    fn test_resolve_app_explicit() {
        assert_eq!(resolve_app(Some("my-app".to_string())).unwrap(), "my-app");
        assert_eq!(
            resolve_app(Some("  custom-app  ".to_string())).unwrap(),
            "custom-app"
        );
    }
    #[test]
    fn test_server_input_normalization_table() {
        // bare v4
        assert_eq!(
            normalize_server_url("43.173.34.155").unwrap(),
            "https://43.173.34.155:8443"
        );
        // bare v6
        assert_eq!(normalize_server_url("::1").unwrap(), "https://[::1]:8443");
        assert_eq!(
            normalize_server_url("2001:db8::1").unwrap(),
            "https://[2001:db8::1]:8443"
        );
        // host:port
        assert_eq!(
            normalize_server_url("43.173.34.155:8443").unwrap(),
            "https://43.173.34.155:8443"
        );
        assert_eq!(
            normalize_server_url("example.com:8443").unwrap(),
            "https://example.com:8443"
        );
        assert_eq!(
            normalize_server_url("[2001:db8::1]:8443").unwrap(),
            "https://[2001:db8::1]:8443"
        );
        // full URL
        assert_eq!(
            normalize_server_url("https://host").unwrap(),
            "https://host"
        );
        assert_eq!(
            normalize_server_url("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            normalize_server_url("https://example.com:9443").unwrap(),
            "https://example.com:9443"
        );
        // hostname
        assert_eq!(normalize_server_url("host").unwrap(), "https://host");
        assert_eq!(
            normalize_server_url("example.com").unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn test_version_compatibility_matrix() {
        // 0.4.0 vs 0.4.0 ok
        assert_eq!(check_version_compatibility("0.4.0", "0.4.0").unwrap(), None);

        // 0.4.0 vs 0.4.1 warn
        let warn = check_version_compatibility("0.4.0", "0.4.1").unwrap();
        assert!(warn.is_some());
        assert!(warn.unwrap().contains("Warning"));

        // 0.4.0 vs 0.5.0 abort
        let err_minor = check_version_compatibility("0.4.0", "0.5.0").unwrap_err();
        assert_eq!(err_minor.code, "version_mismatch");

        // 1.0.0 vs 2.0.0 abort
        let err_major = check_version_compatibility("1.0.0", "2.0.0").unwrap_err();
        assert_eq!(err_major.code, "version_mismatch");
    }

    #[test]
    fn test_init_project_layout() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_init_layout_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<h1>hi</h1>").unwrap();

        let res = init_project(&root, Some("my-cool-app")).unwrap();
        assert_eq!(res.app_name, "my-cool-app");

        assert!(!root.join("skoci.yaml").exists());
        assert!(!root.join("Dockerfile").exists());
        assert!(!root.join(".dockerignore").exists());

        assert!(root.join(PROJECT_DIR).is_dir());
        assert!(root.join(PROJECT_CONFIG_PATH).is_file());
        assert!(root.join(PROJECT_IGNORE_PATH).is_file());
        assert!(root.join(PROJECT_DOCKERFILE_PATH).is_file());

        let gitignore = root.join(".gitignore");
        assert!(gitignore.is_file());
        let gitignore_content = std::fs::read_to_string(&gitignore).unwrap();
        assert!(gitignore_content.lines().any(|l| l.trim() == ".skoci/env"));

        for file in &res.created_files {
            assert!(file.starts_with(".skoci/") || file == ".gitignore");
        }

        let cfg_content = std::fs::read_to_string(root.join(PROJECT_CONFIG_PATH)).unwrap();
        assert!(cfg_content.contains("dockerfile: .skoci/Dockerfile"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_init_keeps_user_dockerfile_at_root() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_init_userdf_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();
        let user_df = "FROM alpine\nEXPOSE 9000\nCMD [\"sleep\", \"1\"]\n";
        std::fs::write(root.join("Dockerfile"), user_df).unwrap();

        init_project(&root, Some("own-recipe")).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("Dockerfile")).unwrap(),
            user_df
        );
        assert!(!root.join(PROJECT_DOCKERFILE_PATH).exists());
        let cfg_content = std::fs::read_to_string(root.join(PROJECT_CONFIG_PATH)).unwrap();
        assert!(cfg_content.contains("dockerfile: Dockerfile"));
        assert!(!cfg_content.contains(".skoci/Dockerfile"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_init_project_legacy_skoci_yaml_preserved() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_init_legacy_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        let legacy_file = root.join("skoci.yaml");
        let legacy_content = "version: 1\napp: legacy-custom-app\n";
        std::fs::write(&legacy_file, legacy_content).unwrap();

        let res = init_project(&root, None).unwrap();
        assert_eq!(res.app_name, "legacy-custom-app");
        assert!(res.skipped_files.contains(&PROJECT_CONFIG_PATH.to_string()));

        // Root skoci.yaml was migrated to .skoci/skoci.yaml and its content preserved
        assert!(!legacy_file.exists());
        let migrated_file = root.join(PROJECT_CONFIG_PATH);
        assert!(migrated_file.is_file());
        let read_back = std::fs::read_to_string(&migrated_file).unwrap();
        assert_eq!(read_back, legacy_content);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_resolve_global_with_local_master_env() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let master_file = temp_dir.join(format!("test_master_{unique_id}.env"));
        let project_root = temp_dir.join(format!("test_empty_proj_{unique_id}"));
        std::fs::create_dir_all(&project_root).unwrap();

        std::fs::write(
            &master_file,
            "SKOCI_MASTER_TOKEN=master_token_secret_12345\n",
        )
        .unwrap();

        let global = GlobalOpts::default();
        let profiles = skoci_client::profile::ProfileConfig::default();
        let target = resolve_global_at(&global, &master_file, &project_root, &profiles).unwrap();

        assert_eq!(target.url, "http://127.0.0.1:8787");
        assert_eq!(target.token.as_deref(), Some("master_token_secret_12345"));
        assert_eq!(
            target.source,
            skoci_client::profile::TargetSource::LocalMaster
        );

        let _ = std::fs::remove_file(master_file);
        let _ = std::fs::remove_dir_all(project_root);
    }
    #[test]
    fn test_validate_project_valid() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_validate_valid_{unique_id}"));
        let skoci_dir = root.join(".skoci");
        std::fs::create_dir_all(&skoci_dir).unwrap();
        let config_yaml = r#"version: 1
app: my-test-app
domains:
  - my-test-app.example.com
preview_domains:
  - preview.example.com
build:
  dockerfile: .skoci/Dockerfile
  context: .
run:
  port: 8080
env:
  secrets:
    - DATABASE_URL
"#;
        std::fs::write(skoci_dir.join("skoci.yaml"), config_yaml).unwrap();

        let res = validate_project(&root).unwrap();
        assert_eq!(res.app, "my-test-app");
        assert_eq!(res.domains, vec!["my-test-app.example.com"]);
        assert_eq!(res.preview_domains, vec!["preview.example.com"]);
        assert_eq!(res.dockerfile, ".skoci/Dockerfile");
        assert_eq!(res.context, ".");
        assert_eq!(res.required_secrets, vec!["DATABASE_URL"]);
        assert!(res.config_path.contains("skoci.yaml"));
        assert!(res.files.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_validate_project_verbose_lists_upload_files() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_validate_verbose_{unique_id}"));
        let skoci_dir = root.join(".skoci");
        std::fs::create_dir_all(&skoci_dir).unwrap();
        let config_yaml = r#"version: 1
app: my-verbose-app
build:
  dockerfile: .skoci/Dockerfile
  context: .
run:
  port: 8080
"#;
        std::fs::write(skoci_dir.join("skoci.yaml"), config_yaml).unwrap();
        std::fs::write(skoci_dir.join("Dockerfile"), "FROM alpine:3.20\n").unwrap();
        std::fs::write(root.join("index.html"), "<h1>hello</h1>\n").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1\n").unwrap();

        let plain = validate_project_with_options(&root, false).unwrap();
        assert!(plain.files.is_none());
        assert!(plain.bytes.is_none());
        assert!(plain.excluded_sensitive.is_none());

        let verbose = validate_project_with_options(&root, true).unwrap();
        let files = verbose.files.expect("files should be populated in verbose mode");
        assert!(files.contains(&"index.html".to_string()));
        assert!(files.contains(&".skoci/skoci.yaml".to_string()));
        assert!(files.contains(&".skoci/Dockerfile".to_string()));
        assert!(!files.contains(&".env".to_string()));
        assert!(verbose.bytes.unwrap() > 0);
        let excluded = verbose.excluded_sensitive.unwrap();
        assert!(excluded.iter().any(|e| e.contains(".env")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_validate_project_invalid() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_validate_invalid_{unique_id}"));
        let skoci_dir = root.join(".skoci");
        std::fs::create_dir_all(&skoci_dir).unwrap();
        let config_yaml = r#"version: 1
app: ""
domains: []
run:
  port: 8080
"#;
        std::fs::write(skoci_dir.join("skoci.yaml"), config_yaml).unwrap();

        let err = validate_project(&root).unwrap_err();
        assert_eq!(err.code, "config_invalid");
        assert!(!err.fix.is_empty());
        assert_eq!(error_exit_code(&err), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_validate_project_missing_config() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_validate_missing_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        let err = validate_project(&root).unwrap_err();
        assert_eq!(err.code, "config_invalid");
        assert!(err.message.contains("skoci.yaml file not found"));
        assert_eq!(error_exit_code(&err), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_init_project_suggest_app_name_from_package_json() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("belajarpaper_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name": "@myorg/pahampaper-id", "version": "1.0.0"}"#,
        )
        .unwrap();

        let res = init_project(&root, None).unwrap();
        assert_eq!(res.app_name, "pahampaper-id");

        let cfg_content = std::fs::read_to_string(root.join(PROJECT_CONFIG_PATH)).unwrap();
        assert!(cfg_content.contains("app: pahampaper-id"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_init_project_unnormalisable_dir_name_early_failure() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let base = temp_dir.join(format!("test_unnormalisable_{unique_id}"));
        let root = base.join("---");
        std::fs::create_dir_all(&root).unwrap();

        let err = init_project(&root, None).unwrap_err();
        assert_eq!(err.code, "config_invalid");
        assert!(err.fix.contains("skoci init --app <name>"));
        assert!(!root.join(".skoci").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_init_project_invalid_explicit_app_early_failure() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_explicit_bad_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        let err = init_project(&root, Some("invalid--name")).unwrap_err();
        assert_eq!(err.code, "config_invalid");
        assert!(err.fix.contains("skoci init --app <name>"));
        assert!(err.fix.contains("--"));
        assert!(!root.join(".skoci").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_connect_credential_store_selection() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_connect_store_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();
        let profile_path = root.join("config").join("servers.json");

        // 1. Default (project_env = false): writes servers.json only, no .skoci/env
        let stores = persist_connect_credentials_at(
            &root,
            &profile_path,
            &ConnectCredentials {
                name: "default",
                server_url: "https://1.2.3.4:8443",
                token: "sk_tok_123",
                pin: Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
                project_env: false,
                no_profile: false,
            },
        )
        .unwrap();
        assert_eq!(stores.len(), 1);
        assert!(stores[0].contains("servers.json"));
        assert!(!root.join(".skoci/env").exists());
        assert!(profile_path.is_file());

        // 2. With --project-env (project_env = true): writes .skoci/env and .gitignore
        let stores_proj = persist_connect_credentials_at(
            &root,
            &profile_path,
            &ConnectCredentials {
                name: "default",
                server_url: "https://1.2.3.4:8443",
                token: "sk_tok_123",
                pin: Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
                project_env: true,
                no_profile: false,
            },
        )
        .unwrap();
        assert_eq!(stores_proj.len(), 2);
        assert!(stores_proj.iter().any(|s| s.contains(".skoci/env")));
        assert!(root.join(".skoci/env").is_file());
        assert!(root.join(".gitignore").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_agent_guide_updated_content() {
        let guide = agent_guide_text();
        assert!(guide.contains("skoci server setup --host <ip> --user <user>"));
        assert!(guide.contains("skoci connect --server https://<ip>:8443 --token <token> --pin sha256:<64 hex>"));
        assert!(guide.contains("~/.config/skoci/servers.json"));
        assert!(guide.contains("FIRST promotion to production"));
        assert!(guide.contains("sudo skoci approve <id>"));
        assert!(guide.contains("SKOCI_REQUIRE_APPROVAL_ALWAYS=true"));
        assert!(guide.contains("--strict-approvals"));
        assert!(guide.contains("/etc/skoci/skoci.env"));
    }
}
