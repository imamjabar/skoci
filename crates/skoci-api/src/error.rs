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
            format!("Ukuran kode yang diunggah melebihi batas {limit_mb} MB"),
            "Tambahkan folder besar (node_modules, dist, data) ke .dockerignore atau .gitignore",
        )
    }

    pub fn rate_limited() -> Self {
        Self::new(
            "rate_limited",
            "Terlalu banyak percobaan token yang salah dari alamat ini",
            "Tunggu 1 menit lalu coba lagi dengan token yang benar",
        )
    }

    pub fn build_failed(details: impl Into<String>) -> Self {
        Self::new(
            "build_failed",
            details,
            "Baca log build di atas, perbaiki Dockerfile atau kode, lalu jalankan 'skoci deploy' lagi",
        )
    }

    pub fn secret_missing(name: &str) -> Self {
        Self::new(
            "secret_missing",
            format!("Secret '{name}' belum diset"),
            format!("skoci secret set {name} <nilai>"),
        )
    }

    pub fn policy_violation(action: &str, env: &str) -> Self {
        Self::new(
            "policy_violation",
            format!("Aksi '{action}' di environment '{env}' dilarang oleh kebijakan server Skoci"),
            "Minta admin server menjalankan aksi ini",
        )
    }

    pub fn approval_required(request_id: &str, action: &str) -> Self {
        Self::new(
            "approval_required",
            format!("Aksi '{action}' memerlukan persetujuan manusia (ID: {request_id})"),
            format!("Minta admin menyetujui lewat dashboard atau 'skoci approve {request_id}'"),
        )
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(
            "unauthorized",
            message,
            "Set SKOCI_TOKEN ke token agen yang valid (skoci connect) atau login dashboard dengan master token",
        )
    }

    pub fn scope_violation(token_app: &str, requested_app: &str) -> Self {
        Self::new(
            "scope_violation",
            format!("Token ini hanya memiliki izin untuk aplikasi '{token_app}', bukan '{requested_app}'"),
            format!("Gunakan token yang dibuat khusus untuk '{requested_app}'"),
        )
    }

    pub fn app_not_found(app: &str) -> Self {
        Self::new(
            "app_not_found",
            format!("Aplikasi '{app}' belum pernah di-deploy"),
            "Jalankan 'skoci deploy' dari folder project",
        )
    }

    pub fn health_check_failed(target: &str, details: &str) -> Self {
        Self::new(
            "health_check_failed",
            format!("Health check gagal untuk {target}: {details}"),
            "Pastikan aplikasi listen di 0.0.0.0 pada 'run.port' dan path 'run.health' membalas 2xx; cek 'skoci logs'",
        )
    }

    pub fn smoke_test_failed(target: &str, details: &str) -> Self {
        Self::new(
            "smoke_test_failed",
            format!("Smoke test gagal untuk {target}: {details}"),
            "Periksa 'skoci logs' atau jalankan 'skoci rollback'",
        )
    }

    pub fn docker_error(message: impl Into<String>) -> Self {
        Self::new(
            "docker_error",
            message,
            "Pastikan Docker berjalan: 'sudo systemctl status docker' atau 'skoci doctor'",
        )
    }
}

impl fmt::Display for SkociError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} (Solusi: {})", self.code, self.message, self.fix)
    }
}

impl std::error::Error for SkociError {}
