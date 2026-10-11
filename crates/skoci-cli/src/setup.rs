use std::net::IpAddr;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{normalize_spki_pin, ApiResponse, CONTROL_TLS_PORT, MAX_APP_NAME_LEN};
use skoci_api::responses::{ServerFingerprint, TokenCreateResponse};

const INSTALLER_SCRIPT: &str = include_str!("../../../install.sh");

#[derive(Debug, Clone)]
pub struct SetupOptions {
    pub host: String,
    pub user: String,
    pub port: u16,
    pub ssh_key: Option<PathBuf>,
    pub app: Option<String>,
    pub version: Option<String>,
    pub magic_dns: bool,
    pub domain: Option<String>,
    pub email: Option<String>,
    pub assume_yes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSetupResponse {
    pub host: String,
    pub control_url: String,
    pub spki_pin: String,
    pub profile: String,
    pub app: String,
    pub next_command: String,
    pub steps: Vec<String>,
}

pub fn is_valid_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host.chars().any(|c| {
        c.is_whitespace()
            || c == ';'
            || c == '/'
            || c == '\\'
            || c == '`'
            || c == '$'
            || c == '&'
            || c == '|'
            || c == '>'
            || c == '<'
    }) {
        return false;
    }
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.is_empty() {
        return false;
    }
    for label in &labels {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
    }
    true
}

pub fn is_valid_user(user: &str) -> bool {
    let user = user.trim();
    if user.is_empty() || user.len() > 32 {
        return false;
    }
    let mut chars = user.chars();
    let first = match chars.next() {
        Some(c) => c.is_ascii_lowercase() || c == '_',
        None => return false,
    };
    if !first {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn is_valid_app_name(name: &str) -> bool {
    skoci_core::config::is_valid_app_name(name)
}

pub fn build_installer_flags(opts: &SetupOptions) -> Vec<String> {
    let mut flags = Vec::new();
    if opts.magic_dns {
        flags.push("--magic-dns".to_string());
    }
    if let Some(domain) = &opts.domain {
        flags.push("--domain".to_string());
        flags.push(domain.clone());
    }
    if let Some(email) = &opts.email {
        flags.push("--email".to_string());
        flags.push(email.clone());
    }
    if let Some(version) = &opts.version {
        flags.push("--version".to_string());
        flags.push(version.clone());
    }
    flags
}

pub fn build_ssh_args(opts: &SetupOptions, remote_argv: &[&str]) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-p".to_string(),
        opts.port.to_string(),
    ];
    if let Some(key) = &opts.ssh_key {
        args.push("-i".to_string());
        args.push(key.display().to_string());
    }
    args.push(format!("{}@{}", opts.user, opts.host));
    for arg in remote_argv {
        args.push(arg.to_string());
    }
    args
}

fn parse_fingerprint_response(stdout: &str) -> Result<ServerFingerprint, SkociError> {
    if let Ok(resp) = serde_json::from_str::<ApiResponse<ServerFingerprint>>(stdout) {
        if let Some(fp) = resp.data {
            return Ok(fp);
        }
        if let Some(err) = resp.error {
            return Err(err);
        }
    }
    if let Ok(fp) = serde_json::from_str::<ServerFingerprint>(stdout) {
        return Ok(fp);
    }
    if let Some(start) = stdout.find('{') {
        if let Some(end) = stdout.rfind('}') {
            let slice = &stdout[start..=end];
            if let Ok(resp) = serde_json::from_str::<ApiResponse<ServerFingerprint>>(slice) {
                if let Some(fp) = resp.data {
                    return Ok(fp);
                }
            }
            if let Ok(fp) = serde_json::from_str::<ServerFingerprint>(slice) {
                return Ok(fp);
            }
        }
    }
    Err(SkociError::internal(
        format!("Failed to parse server fingerprint from response: {stdout}"),
        "Check that 'skoci server fingerprint --json' outputs valid JSON on the remote server",
    ))
}

fn parse_token_create_response(stdout: &str) -> Result<TokenCreateResponse, SkociError> {
    if let Ok(resp) = serde_json::from_str::<ApiResponse<TokenCreateResponse>>(stdout) {
        if let Some(t) = resp.data {
            return Ok(t);
        }
        if let Some(err) = resp.error {
            return Err(err);
        }
    }
    if let Ok(t) = serde_json::from_str::<TokenCreateResponse>(stdout) {
        return Ok(t);
    }
    if let Some(start) = stdout.find('{') {
        if let Some(end) = stdout.rfind('}') {
            let slice = &stdout[start..=end];
            if let Ok(resp) = serde_json::from_str::<ApiResponse<TokenCreateResponse>>(slice) {
                if let Some(t) = resp.data {
                    return Ok(t);
                }
            }
            if let Ok(t) = serde_json::from_str::<TokenCreateResponse>(slice) {
                return Ok(t);
            }
        }
    }
    Err(SkociError::internal(
        format!("Failed to parse token create response: {stdout}"),
        "Verify that 'skoci token create' outputs valid JSON",
    ))
}

async fn run_remote_skoci_cmd(opts: &SetupOptions, subcmd: &[&str]) -> Result<String, SkociError> {
    let mut remote_argv = Vec::new();
    if opts.user != "root" {
        remote_argv.push("sudo");
    }
    remote_argv.push("skoci");
    remote_argv.extend_from_slice(subcmd);

    let args = build_ssh_args(opts, &remote_argv);
    let output = tokio::process::Command::new("ssh")
        .args(&args)
        .output()
        .await
        .map_err(|e| {
            SkociError::internal(
                format!("Failed to execute remote command over SSH: {e}"),
                "Ensure OpenSSH client ('ssh') is available in PATH",
            )
        })?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    if opts.user != "root" {
        let mut fallback_argv = vec!["skoci"];
        fallback_argv.extend_from_slice(subcmd);
        let fallback_args = build_ssh_args(opts, &fallback_argv);
        let fallback_output = tokio::process::Command::new("ssh")
            .args(&fallback_args)
            .output()
            .await
            .map_err(|e| {
                SkociError::internal(
                    format!("Failed to execute fallback remote command: {e}"),
                    "Ensure OpenSSH client ('ssh') is available in PATH",
                )
            })?;
        if fallback_output.status.success() {
            return Ok(String::from_utf8_lossy(&fallback_output.stdout).to_string());
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(SkociError::internal(
        format!(
            "Remote command failed (exit code {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ),
        "Ensure Skoci is installed on the server and the user has permissions to run it",
    ))
}

async fn do_server_setup(opts: SetupOptions, json: bool) -> SkociResult<ServerSetupResponse> {
    let mut steps = Vec::new();

    // Step 1: Validate inputs
    let step1_msg = "Step 1/7: Validating server and application inputs".to_string();
    if !json {
        eprintln!("▸ [1/7] Validating server and application inputs...");
    }
    steps.push(step1_msg);

    if !is_valid_host(&opts.host) {
        return Err(SkociError::bad_request(
            format!("Invalid host '{}'", opts.host),
            "Provide an IP literal (e.g. 203.0.113.10) or a syntactically valid hostname (e.g. vps.example.com)",
        ));
    }

    if !is_valid_user(&opts.user) {
        return Err(SkociError::bad_request(
            format!("Invalid SSH username '{}'", opts.user),
            "Provide a valid Linux username matching '^[a-z_][a-z0-9_-]*$'",
        ));
    }

    let app = match &opts.app {
        Some(a) => {
            let trimmed = a.trim();
            if !is_valid_app_name(trimmed) {
                return Err(SkociError::bad_request(
                    format!("Invalid app name '{trimmed}'"),
                    format!(
                        "Use lowercase alphanumeric characters and hyphens, up to {MAX_APP_NAME_LEN} characters"
                    ),
                ));
            }
            trimmed.to_string()
        }
        None => {
            let yaml_path = Path::new("skoci.yaml");
            if yaml_path.exists() {
                match skoci_core::config::ConfigLoader::load_config(yaml_path) {
                    Ok(cfg) => {
                        let trimmed = cfg.app.trim();
                        if !is_valid_app_name(trimmed) {
                            return Err(SkociError::bad_request(
                                format!("Invalid app name '{trimmed}' in .skoci/skoci.yaml"),
                                format!(
                                    "Use lowercase alphanumeric characters and hyphens, up to {MAX_APP_NAME_LEN} characters"
                                ),
                            ));
                        }
                        trimmed.to_string()
                    }
                    Err(e) => {
                        return Err(SkociError::bad_request(
                            format!("Failed to read skoci.yaml: {e}"),
                            "Specify --app <name> explicitly or fix skoci.yaml in the current directory",
                        ));
                    }
                }
            } else {
                return Err(SkociError::bad_request(
                    "App name not specified and skoci.yaml not found in current directory",
                    "Specify --app <name> or run 'skoci init' in your project directory first",
                ));
            }
        }
    };

    if let Some(d) = &opts.domain {
        if !is_valid_host(d) {
            return Err(SkociError::bad_request(
                format!("Invalid domain '{d}'"),
                "Provide a syntactically valid domain name",
            ));
        }
    }

    if let Some(e) = &opts.email {
        if e.is_empty() || !e.contains('@') || e.contains(' ') {
            return Err(SkociError::bad_request(
                format!("Invalid email address '{e}'"),
                "Provide a valid email address for ACME certificate registration",
            ));
        }
    }

    // Step 2: Preflight SSH connection
    let step2_msg = format!("Step 2/7: Testing SSH connectivity to {}@{}", opts.user, opts.host);
    if !json {
        eprintln!(
            "▸ [2/7] Testing SSH connectivity to {}@{}:{}...",
            opts.user, opts.host, opts.port
        );
    }
    steps.push(step2_msg);

    let mut preflight_cmd = tokio::process::Command::new("ssh");
    preflight_cmd
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-p")
        .arg(opts.port.to_string());
    if let Some(key) = &opts.ssh_key {
        preflight_cmd.arg("-i").arg(key);
    }
    preflight_cmd.arg(format!("{}@{}", opts.user, opts.host));
    preflight_cmd.arg("true");

    let preflight_output = preflight_cmd.output().await.map_err(|e| {
        SkociError::internal(
            format!("Failed to execute SSH preflight: {e}"),
            "Ensure OpenSSH client ('ssh') is installed and accessible in PATH",
        )
    })?;

    if !preflight_output.status.success() {
        let stderr = String::from_utf8_lossy(&preflight_output.stderr);
        let stderr_lower = stderr.to_ascii_lowercase();
        if stderr_lower.contains("permission denied")
            || stderr_lower.contains("publickey")
            || stderr_lower.contains("authentication failed")
        {
            return Err(SkociError::bad_request(
                format!(
                    "SSH authentication failed for {}@{}:{}",
                    opts.user, opts.host, opts.port
                ),
                "Password authentication requires an interactive terminal (sshpass is not supported by Skoci). Please add an SSH key using 'ssh-copy-id' or specify '--ssh-key <path>', or run the installer manually on the VPS.",
            ));
        }
        if stderr_lower.contains("host key verification failed") {
            return Err(SkociError::bad_request(
                format!("SSH host key verification failed for {}", opts.host),
                "Check your ~/.ssh/known_hosts file or update the remote server host key",
            ));
        }
        return Err(SkociError::bad_request(
            format!(
                "Failed SSH connection to {}@{}:{}: {}",
                opts.user,
                opts.host,
                opts.port,
                stderr.trim()
            ),
            "Verify the host address, port, and that the SSH server is running and accessible",
        ));
    }

    let target_host = if opts.port == 22 {
        opts.host.clone()
    } else {
        format!("[{}]:{}", opts.host, opts.port)
    };
    let mut kg_cmd = tokio::process::Command::new("ssh-keygen");
    kg_cmd.arg("-l").arg("-F").arg(&target_host);
    if let Ok(out) = kg_cmd.output().await {
        let out_str = String::from_utf8_lossy(&out.stdout);
        let fp_line = out_str
            .lines()
            .find(|line| !line.starts_with('#') && line.contains("SHA256:"));
        if let Some(line) = fp_line {
            if let Some(fp_token) = line.split_whitespace().find(|w| w.starts_with("SHA256:")) {
                if !json {
                    eprintln!("  Host key fingerprint: {fp_token}");
                }
            }
        }
    }

    // Step 3: Run installer remotely
    let step3_msg = "Step 3/7: Running installer remotely".to_string();
    if !json {
        eprintln!("▸ [3/7] Running Skoci installer remotely (streaming progress)...");
    }
    steps.push(step3_msg);

    let installer_flags = build_installer_flags(&opts);
    let mut install_cmd = tokio::process::Command::new("ssh");
    install_cmd
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-p")
        .arg(opts.port.to_string());
    if let Some(key) = &opts.ssh_key {
        install_cmd.arg("-i").arg(key);
    }
    install_cmd.arg(format!("{}@{}", opts.user, opts.host));
    install_cmd.arg("sh");
    install_cmd.arg("-s");
    install_cmd.arg("--");
    for flag in &installer_flags {
        install_cmd.arg(flag);
    }

    install_cmd.stdin(std::process::Stdio::piped());
    install_cmd.stdout(std::process::Stdio::piped());
    install_cmd.stderr(std::process::Stdio::piped());

    let mut child = install_cmd.spawn().map_err(|e| {
        SkociError::internal(
            format!("Failed to spawn SSH process for installer: {e}"),
            "Ensure OpenSSH client ('ssh') is installed and accessible in PATH",
        )
    })?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(INSTALLER_SCRIPT.as_bytes())
            .await
            .map_err(|e| {
                SkociError::internal(
                    format!("Failed to feed installer script to remote shell: {e}"),
                    "Check SSH connection and remote shell availability",
                )
            })?;
        drop(stdin);
    }

    use tokio::io::AsyncBufReadExt;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_task = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("  | {line}");
            }
        }
    });

    let stderr_task = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("  | {line}");
            }
        }
    });

    let _ = tokio::join!(stdout_task, stderr_task);

    let status = child.wait().await.map_err(|e| {
        SkociError::internal(
            format!("Failed waiting for remote installer: {e}"),
            "Check network connection to the server",
        )
    })?;

    if !status.success() {
        return Err(SkociError::internal(
            format!(
                "Remote installation failed with exit code {}",
                status.code().unwrap_or(-1)
            ),
            "Inspect the installer output above for the specific error, or run the installer manually on the VPS",
        ));
    }

    // Step 4: Read server fingerprint over SSH
    let step4_msg = "Step 4/7: Retrieving server TLS fingerprint".to_string();
    if !json {
        eprintln!("▸ [4/7] Retrieving server TLS fingerprint...");
    }
    steps.push(step4_msg);

    let fp_stdout = run_remote_skoci_cmd(&opts, &["server", "fingerprint", "--json"]).await?;
    let fp = parse_fingerprint_response(&fp_stdout)?;

    let pin = match normalize_spki_pin(&fp.spki_pin) {
        Some(p) => p,
        None => {
            return Err(SkociError::internal(
                format!("Remote server reported an invalid SPKI pin: '{}'", fp.spki_pin),
                "Ensure 'skoci server' generated /var/lib/skoci/tls/control.pin correctly on the VPS",
            ));
        }
    };

    let control_url = fp.control_urls.first().cloned().unwrap_or_else(|| {
        let host = &opts.host;
        if host.contains(':') && !host.starts_with('[') {
            format!("https://[{host}]:{CONTROL_TLS_PORT}")
        } else {
            format!("https://{host}:{CONTROL_TLS_PORT}")
        }
    });

    // Step 5: Create scoped agent token over SSH
    let step5_msg = "Step 5/7: Creating scoped agent token".to_string();
    if !json {
        eprintln!("▸ [5/7] Creating scoped agent token for '{app}'...");
    }
    steps.push(step5_msg);

    let token_stdout = run_remote_skoci_cmd(
        &opts,
        &["token", "create", "--app", &app, "--name", "laptop", "--json"],
    )
    .await?;
    let token_resp = parse_token_create_response(&token_stdout)?;

    // Step 6: Save profile & verify connection
    let step6_msg = "Step 6/7: Storing profile and verifying pinned connection".to_string();
    if !json {
        eprintln!("▸ [6/7] Storing profile and verifying pinned connection...");
    }
    steps.push(step6_msg);

    skoci_client::profile::upsert_profile(
        &opts.host,
        &control_url,
        &token_resp.token,
        Some(&pin),
    )?;
    skoci_client::profile::set_current_profile(&opts.host)?;

    let client = skoci_client::SkociClient::new(&control_url, Some(token_resp.token), Some(pin.clone()))?;
    let doctor_resp = client.doctor().await.map_err(|e| {
        SkociError::internal(
            format!("Failed to verify connection to {control_url}: {e}"),
            "Check that the Skoci control server is running and TLS certificate is valid",
        )
    })?;

    if !doctor_resp.success {
        let msg = if !doctor_resp.summary.is_empty() {
            &doctor_resp.summary
        } else if let Some(err) = &doctor_resp.error {
            &err.message
        } else {
            "Doctor check failed"
        };
        return Err(SkociError::internal(
            format!("Connection verification failed: {msg}"),
            "Check remote server logs using 'journalctl -u skoci'",
        ));
    }

    // Step 7: Final summary
    let step7_msg = "Step 7/7: Setup completed successfully".to_string();
    steps.push(step7_msg);

    Ok(ServerSetupResponse {
        host: opts.host.clone(),
        control_url,
        spki_pin: pin,
        profile: opts.host,
        app,
        next_command: "skoci deploy".to_string(),
        steps,
    })
}

pub async fn run_server_setup(opts: SetupOptions, json: bool) -> i32 {
    match do_server_setup(opts, json).await {
        Ok(resp) => {
            if json {
                crate::output::print_json(&ApiResponse::ok(resp, "Server setup completed successfully"));
            } else {
                eprintln!("\n✓ Skoci server setup completed successfully!\n");
                println!("Control URL : {}", resp.control_url);
                println!("SPKI Pin    : {}", resp.spki_pin);
                println!("Profile     : {} (saved and set as current)", resp.profile);
                println!("Application : {}", resp.app);
                println!("\nNext steps:");
                println!("  Deploy your app: {}", resp.next_command);
            }
            0
        }
        Err(err) => {
            crate::output::print_error(&err, json);
            1
        }
    }
}

pub async fn run_server_fingerprint(json: bool) -> i32 {
    let data_dir = std::env::var("SKOCI_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/skoci"));

    let pin_path = data_dir.join("tls").join("control.pin");
    if !pin_path.exists() {
        let err = SkociError::not_found(
            format!("Control TLS pin file not found at '{}'", pin_path.display()),
            "Run 'skoci server' on the VPS first to generate the server key and pin",
        );
        crate::output::print_error(&err, json);
        return 1;
    }

    let pin_raw = match std::fs::read_to_string(&pin_path) {
        Ok(s) => s,
        Err(e) => {
            let err = SkociError::internal(
                format!("Failed to read pin file '{}': {e}", pin_path.display()),
                "Check file permissions on /var/lib/skoci/tls/control.pin",
            );
            crate::output::print_error(&err, json);
            return 1;
        }
    };

    let pin = match normalize_spki_pin(pin_raw.trim()) {
        Some(p) => p,
        None => {
            let err = SkociError::internal(
                format!("Invalid pin in '{}': '{}'", pin_path.display(), pin_raw.trim()),
                "Regenerate server TLS credentials by removing the invalid pin and restarting skoci server",
            );
            crate::output::print_error(&err, json);
            return 1;
        }
    };

    let mut control_urls = Vec::new();
    let public_ip = std::env::var("SKOCI_PUBLIC_IP")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            let hint = data_dir.join("tls").join("public_ip");
            std::fs::read_to_string(hint)
                .ok()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
        })
        .or_else(|| {
            let content = std::fs::read_to_string(Path::new("/etc/skoci/skoci.env")).ok()?;
            crate::env::parse_env_file_content(&content)
                .into_iter()
                .find(|(k, v)| k == "SKOCI_PUBLIC_IP" && !v.trim().is_empty())
                .map(|(_, v)| v)
        });

    if let Some(ip) = public_ip {
        let url = if ip.contains(':') && !ip.starts_with('[') {
            format!("https://[{ip}]:{CONTROL_TLS_PORT}")
        } else {
            format!("https://{ip}:{CONTROL_TLS_PORT}")
        };
        control_urls.push(url);
    }

    let fp = ServerFingerprint {
        spki_pin: pin,
        control_urls,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    if json {
        match serde_json::to_string_pretty(&fp) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                let err = SkociError::internal(
                    format!("Failed to serialize server fingerprint: {e}"),
                    "Internal JSON serialization failure",
                );
                crate::output::print_error(&err, true);
                return 1;
            }
        }
    } else {
        println!("SPKI Pin: {}", fp.spki_pin);
        for url in &fp.control_urls {
            println!("Control URL: {url}");
        }
        println!("Version: {}", fp.version);
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_validation() {
        assert!(is_valid_host("203.0.113.10"));
        assert!(is_valid_host("2001:db8::1"));
        assert!(is_valid_host("vps.example.com"));

        assert!(!is_valid_host("a b"));
        assert!(!is_valid_host("-bad.com"));
        assert!(!is_valid_host("host;rm -rf /"));
        assert!(!is_valid_host(""));
    }

    #[test]
    fn test_user_validation() {
        assert!(is_valid_user("root"));
        assert!(is_valid_user("deploy_user"));
        assert!(is_valid_user("user-1"));
        assert!(is_valid_user("_runner"));

        assert!(!is_valid_user("root;x"));
        assert!(!is_valid_user(".."));
        assert!(!is_valid_user(""));
        assert!(!is_valid_user("1user"));
        assert!(!is_valid_user("User"));
    }

    #[test]
    fn test_installer_flag_construction() {
        let default_opts = SetupOptions {
            host: "203.0.113.10".into(),
            user: "root".into(),
            port: 22,
            ssh_key: None,
            app: None,
            version: None,
            magic_dns: false,
            domain: None,
            email: None,
            assume_yes: false,
        };
        assert_eq!(build_installer_flags(&default_opts), Vec::<String>::new());

        let mut magic_opts = default_opts.clone();
        magic_opts.magic_dns = true;
        assert_eq!(build_installer_flags(&magic_opts), vec!["--magic-dns"]);

        let mut domain_email_opts = default_opts.clone();
        domain_email_opts.domain = Some("example.com".into());
        domain_email_opts.email = Some("admin@example.com".into());
        assert_eq!(
            build_installer_flags(&domain_email_opts),
            vec!["--domain", "example.com", "--email", "admin@example.com"]
        );

        let mut version_opts = default_opts.clone();
        version_opts.version = Some("v0.4.0".into());
        assert_eq!(
            build_installer_flags(&version_opts),
            vec!["--version", "v0.4.0"]
        );
    }

    #[test]
    fn test_remote_command_builder_no_space_joined_strings() {
        let opts = SetupOptions {
            host: "203.0.113.10".into(),
            user: "root".into(),
            port: 22,
            ssh_key: None,
            app: Some("myapp".into()),
            version: None,
            magic_dns: false,
            domain: None,
            email: None,
            assume_yes: false,
        };
        let cmd_parts = ["sudo", "skoci", "server", "fingerprint", "--json"];
        let args = build_ssh_args(&opts, &cmd_parts);

        for part in &cmd_parts {
            assert!(args.iter().any(|a| a == part));
        }

        let target = format!("{}@{}", opts.user, opts.host);
        let idx = args.iter().position(|a| a == &target).expect("user@host found");
        let remote_args = &args[idx + 1..];
        for arg in remote_args {
            assert!(!arg.contains(' '), "Remote arg '{arg}' contains a space");
        }
        assert!(!args.iter().any(|a| a.contains("skoci server fingerprint")));
    }
}
