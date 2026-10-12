use serde::{Deserialize, Serialize};
use skoci_api::error::SkociError;
use skoci_api::models::ApiResponse;
use skoci_api::responses::{
    ApprovalInfo, DeploymentInfo, DoctorResponse, DomainCheckResponse, LogsResponse,
    StatusResponse, TokenInfo,
};

pub fn print_json<T: Serialize>(resp: &ApiResponse<T>) {
    match serde_json::to_string_pretty(resp) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Failed to format JSON: {e}"),
    }
}

pub fn print_error(err: &SkociError, is_json: bool) {
    if is_json {
        let resp = ApiResponse::<()>::err(err.clone());
        print_json(&resp);
    } else {
        eprintln!("✗ Error [{}]: {}", err.code, err.message);
        eprintln!("  Fix: {}", err.fix);
    }
}

pub fn render_status(status: &StatusResponse) {
    println!("App: {}", status.app);
    println!("Default App: {}", if status.is_default_app { "Yes" } else { "No" });

    if let Some(dep_id) = &status.running_deployment {
        println!("Running deployment: {dep_id}");
    }

    println!("\nEnvironments:");
    if status.environments.is_empty() {
        println!("  (No environments deployed yet)");
    } else {
        println!("  {:<12} {:<18} {:<12} {:<8} URL", "ENV", "VERSION", "STATUS", "HEALTHY");
        println!("  {}", "-".repeat(70));
        for env in &status.environments {
            let healthy_str = if env.healthy { "✓" } else { "✗" };
            let urls_str = if env.urls.is_empty() {
                "-".to_string()
            } else {
                env.urls.join(", ")
            };
            println!(
                "  {:<12} {:<18} {:<12} {:<8} {}",
                env.env, env.version, env.container_status, healthy_str, urls_str
            );
        }
    }

    if !status.services.is_empty() {
        println!("\nConnected Services:");
        for (service, state) in &status.services {
            println!("  • {}: {}", service, state);
        }
    }
}

pub fn render_deployments(deployments: &[DeploymentInfo]) {
    if deployments.is_empty() {
        println!("No deployment history yet.");
        return;
    }

    println!("{:<24} {:<16} {:<12} {:<18} {:<10} CREATED", "ID", "APP", "ENV", "VERSION", "STATUS");
    println!("{}", "-".repeat(95));
    for dep in deployments {
        let ts = dep.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        println!(
            "{:<24} {:<16} {:<12} {:<18} {:<10} {}",
            dep.id, dep.app, dep.env, dep.version, dep.status, ts
        );
    }
}

pub fn render_approvals(approvals: &[ApprovalInfo]) {
    if approvals.is_empty() {
        println!("No pending approval requests.");
        return;
    }

    println!("{:<24} {:<16} {:<12} {:<10} {:<18} DETAIL", "REQUEST ID", "APP", "ACTION", "STATUS", "REQUESTER");
    println!("{}", "-".repeat(95));
    for app in approvals {
        println!(
            "{:<24} {:<16} {:<12} {:<10} {:<18} {}",
            app.id, app.app, app.action, app.status, app.requested_by, app.detail
        );
    }
}

pub fn render_tokens(tokens: &[TokenInfo]) {
    if tokens.is_empty() {
        println!("No tokens for this app yet.");
        return;
    }

    println!("{:<16} {:<16} {:<16} {:<20} LAST USED", "ID (12 HEX)", "APP", "NAME", "CREATED");
    println!("{}", "-".repeat(85));
    for tok in tokens {
        let created = tok.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let last_used = tok
            .last_used_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<16} {:<16} {:<16} {:<20} {}",
            tok.id, tok.app, tok.name, created, last_used
        );
    }
}

pub fn render_doctor(doctor: &DoctorResponse) {
    let health_str = if doctor.healthy { "Healthy" } else { "Issues Detected" };
    println!("Server Status : {}", health_str);
    println!("Server Version: {}", doctor.version);
    println!("Idle RAM      : {:.1} MB", doctor.idle_ram_mb);
    println!("\nCheck Results:");

    for check in &doctor.checks {
        let icon = if check.passed { "✓" } else { "✗" };
        println!("  {} {:<24} : {}", icon, check.name, check.message);
        if let Some(fix) = &check.fix {
            if !check.passed {
                println!("    Fix: {}", fix);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResponse {
    pub config_path: String,
    pub app: String,
    pub domains: Vec<String>,
    pub preview_domains: Vec<String>,
    pub dockerfile: String,
    pub context: String,
    pub required_secrets: Vec<String>,
}

pub fn render_validate(val: &ValidateResponse) {
    println!("Configuration : Valid");
    println!("Config Path   : {}", val.config_path);
    println!("App Name      : {}", val.app);
    println!("Dockerfile    : {}", val.dockerfile);
    println!("Build Context : {}", val.context);

    println!("\nDomains:");
    if val.domains.is_empty() {
        println!("  Production : (none)");
    } else {
        println!("  Production : {}", val.domains.join(", "));
    }
    if val.preview_domains.is_empty() {
        println!("  Preview    : (none)");
    } else {
        println!("  Preview    : {}", val.preview_domains.join(", "));
    }

    println!("\nRequired Secrets:");
    if val.required_secrets.is_empty() {
        println!("  (none declared)");
    } else {
        for secret in &val.required_secrets {
            println!("  • {}", secret);
        }
    }
}

pub fn sanitize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\x1b' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            match chars[i] {
                '[' => {
                    i += 1;
                    while i < chars.len() {
                        let c = chars[i];
                        if (c as u32) >= 0x40 && (c as u32) <= 0x7e {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                ']' => {
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '\x07' {
                            i += 1;
                            break;
                        }
                        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                '(' | ')' | '*' | '+' => {
                    i += 2;
                }
                _ => {
                    i += 1;
                }
            }
        } else {
            let u = ch as u32;
            if ch == '\t' {
                out.push(ch);
            } else if (u < 0x20) || u == 0x7f || (0x80..=0x9f).contains(&u) {
                // Strip C0 and C1 control character
            } else {
                out.push(ch);
            }
            i += 1;
        }
    }
    out
}

pub fn render_logs(logs: &LogsResponse) {
    println!("App Logs: {} ({}) Container: {}", logs.app, logs.env, logs.container);
    println!("{}", "-".repeat(70));
    for line in &logs.lines {
        println!("{}", sanitize_log_line(line));
    }

    if !logs.error_summary.is_empty() {
        println!("\nError Summary ({} lines):", logs.error_summary.len());
        println!("{}", "-".repeat(70));
        for err_line in &logs.error_summary {
            println!("  [!] {}", sanitize_log_line(err_line));
        }
    }
}
pub fn render_domain_check(check: &DomainCheckResponse) {
    if check.points_here {
        println!("✓ Host '{}' correctly points to this server", check.host);
    } else {
        println!("✗ Host '{}' does NOT point to this server", check.host);
    }
    let exp = if check.expected_ips.is_empty() {
        "(none)".to_string()
    } else {
        check.expected_ips.join(", ")
    };
    let res = if check.resolved_ips.is_empty() {
        "(none)".to_string()
    } else {
        check.resolved_ips.join(", ")
    };
    println!("  Expected IPs: {exp}");
    println!("  Resolved IPs: {res}");
    if check.magic_dns {
        println!("  Magic DNS:    Yes (preview only)");
    }
    if let Some(hint) = &check.record_hint {
        println!("\nDNS Action Required:\n  {hint}");
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_log_line() {
        let raw = "\x1b[31;1mError:\x1b[0m \x1b]52;c;evil\x07failed to connect\r\x1b[2Kspoofed line\twith tab\x00\x08\u{0084}";
        let clean = sanitize_log_line(raw);
        assert!(!clean.contains("\x1b"));
        assert!(!clean.contains("\x07"));
        assert!(!clean.contains("\r"));
        assert!(!clean.contains("\x00"));
        assert!(!clean.contains("\x08"));
        assert!(!clean.contains('\u{0084}'));
        assert!(clean.contains("Error: "));
        assert!(clean.contains("failed to connect"));
        assert!(clean.contains("spoofed line\twith tab"));
    }
}
