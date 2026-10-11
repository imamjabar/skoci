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
            "bad_request" | "config_invalid" | "secret_missing" => 400,
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
}

impl fmt::Display for SkociError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} (Fix: {})", self.code, self.message, self.fix)
    }
}

impl std::error::Error for SkociError {}
