use serde::{Deserialize, Serialize};
use std::fmt;

pub type SkociResult<T> = Result<T, SkociError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkociError {
    pub code: String,
    pub message: String,
    pub fix: String,
}

impl SkociError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            fix: fix.into(),
        }
    }

    /// HTTP status the control plane answers with for this error code.
    pub fn http_status(&self) -> u16 {
        match self.code.as_str() {
            "bad_request" | "config_invalid" | "secret_missing" | "production_requires_domain" | "dns_mismatch"
            | "pin_required" => 400,
            "unauthorized" => 401,
            "forbidden" | "scope_violation" | "policy_violation" | "approval_required" => 403,
            "not_found" | "app_not_found" => 404,
            "conflict" => 409,
            "payload_too_large" => 413,
            "rate_limited" => 429,
            "build_failed" | "health_check_failed" | "smoke_test_failed" => 422,
            "docker_error" => 503,
            _ => 500,
        }
    }

    pub fn bad_request(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("bad_request", message, fix)
    }

    pub fn config_invalid(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("config_invalid", message, fix)
    }

    pub fn internal(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("internal", message, fix)
    }

    pub fn not_found(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("not_found", message, fix)
    }

    pub fn conflict(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("conflict", message, fix)
    }

    pub fn forbidden(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("forbidden", message, fix)
    }

    pub fn payload_too_large(limit_mb: u64) -> Self {
        Self::new(
            "payload_too_large",
            format!("Uploaded code size exceeds the {limit_mb} MB limit"),
            "Add large directories (node_modules, dist, data) to .dockerignore or .gitignore",
        )
    }

    pub fn rate_limited() -> Self {
        Self::new(
            "rate_limited",
            "Too many invalid token attempts from this address",
            "Wait 1 minute and try again with the correct token",
        )
    }

    pub fn build_failed(details: impl Into<String>) -> Self {
        Self::new(
            "build_failed",
            details,
            "Read the build log above, fix the Dockerfile or code, then run 'skoci deploy' again",
        )
    }

    pub fn secret_missing(name: &str) -> Self {
        Self::new(
            "secret_missing",
            format!("Secret '{name}' is not set"),
            format!("skoci secret set {name} <value>"),
        )
    }

    pub fn policy_violation(action: &str, env: &str) -> Self {
        Self::new(
            "policy_violation",
            format!("Action '{action}' in environment '{env}' is forbidden by Skoci server policy"),
            "Ask a server admin to perform this action",
        )
    }

    pub fn approval_required(request_id: &str, action: &str) -> Self {
        Self::new(
            "approval_required",
            format!("Action '{action}' requires human approval (ID: {request_id})"),
            format!("Ask an admin to approve via dashboard or 'skoci approve {request_id}'"),
        )
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(
            "unauthorized",
            message,
            "Set SKOCI_TOKEN to a valid agent token (skoci connect) or log in to the dashboard with the master token",
        )
    }

    pub fn scope_violation(token_app: &str, requested_app: &str) -> Self {
        Self::new(
            "scope_violation",
            format!("This token only has permission for app '{token_app}', not '{requested_app}'"),
            format!("Use a token created specifically for '{requested_app}'"),
        )
    }

    pub fn app_not_found(app: &str) -> Self {
        Self::new(
            "app_not_found",
            format!("App '{app}' has never been deployed"),
            "Run 'skoci deploy' from the project directory",
        )
    }

    pub fn health_check_failed(target: &str, details: &str) -> Self {
        Self::new(
            "health_check_failed",
            format!("Health check failed for {target}: {details}"),
            "Ensure the app listens on 0.0.0.0 at 'run.port' and 'run.health' responds with 2xx; check 'skoci logs'",
        )
    }

    pub fn smoke_test_failed(target: &str, details: &str) -> Self {
        Self::new(
            "smoke_test_failed",
            format!("Smoke test failed for {target}: {details}"),
            "Check 'skoci logs' or run 'skoci rollback'",
        )
    }

    pub fn docker_error(message: impl Into<String>) -> Self {
        Self::new(
            "docker_error",
            message,
            "Ensure Docker is running: 'sudo systemctl status docker' or 'skoci doctor'",
        )
    }

    /// Production hostnames must be owned by the operator; magic-DNS zones (sslip.io, nip.io, ...) are third-party.
    pub fn production_requires_domain(app: &str, rejected: &[String]) -> Self {
        let detail = if rejected.is_empty() {
            String::new()
        } else {
            format!(" (magic-DNS hosts rejected: {})", rejected.join(", "))
        };
        Self::new(
            "production_requires_domain",
            format!("Production for '{app}' needs a domain you own{detail}"),
            "Point an A record at the server, add it under 'domains' in .skoci/skoci.yaml (skoci domain add <host>), then deploy again",
        )
    }

    pub fn dns_mismatch(host: &str, expected: &[String], resolved: &[String]) -> Self {
        let got = if resolved.is_empty() { "nothing".to_string() } else { resolved.join(", ") };
        let fix = match expected.first() {
            Some(ip) if ip.contains(':') => format!("Create DNS record: AAAA {host} -> {ip} (DNS-only, no CDN proxy), wait for propagation, retry"),
            Some(ip) => format!("Create DNS record: A {host} -> {ip} (DNS-only, no CDN proxy), wait for propagation, retry"),
            None => "Point the domain at this server's public IP, then retry".to_string(),
        };
        Self::new(
            "dns_mismatch",
            format!("{host} resolves to {got}, not to this server ({})", expected.join(", ")),
            fix,
        )
    }

    pub fn pin_required(server: &str) -> Self {
        Self::new(
            "pin_required",
            format!("Server {server} is addressed by IP and has no pinned TLS key"),
            "Run 'skoci server setup --host <ip> --user <user>' or pass '--pin sha256:...' from 'sudo skoci server fingerprint' on the VPS",
        )
    }

    pub fn pin_mismatch(server: &str, expected: &str, got: &str) -> Self {
        Self::new(
            "pin_mismatch",
            format!("TLS key of {server} does not match the pinned key (expected {expected}, got {got})"),
            "Possible interception: do not continue. If the server key was rotated on purpose, re-run 'skoci server setup' or reconnect with the new '--pin'",
        )
    }

    pub fn version_mismatch(cli: &str, server: &str) -> Self {
        Self::new(
            "version_mismatch",
            format!("CLI {cli} is incompatible with server {server}"),
            format!("Install the matching CLI: curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/v{server}/install.sh | bash -s -- --client-only --version v{server}"),
        )
    }
}

impl fmt::Display for SkociError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} (Fix: {})", self.code, self.message, self.fix)
    }
}

impl std::error::Error for SkociError {}
