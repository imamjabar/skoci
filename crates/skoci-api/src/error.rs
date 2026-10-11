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

    pub fn bad_request(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("bad_request", message, fix)
    }

    pub fn config_invalid(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new("config_invalid", message, fix)
    }

    pub fn secret_missing(name: &str) -> Self {
        Self::new(
            "secret_missing",
            format!("Secret '{name}' belum diset"),
            format!("skoci secret set {name}"),
        )
    }

    pub fn policy_violation(action: &str, env: &str) -> Self {
        Self::new(
            "policy_violation",
            format!("Aksi '{action}' di environment '{env}' dilarang oleh skoci.policy.yaml"),
            "Perbarui skoci.policy.yaml atau ajukan permohonan izin",
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
            "Gunakan token agen yang valid atau login sebagai manusia",
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
            format!("Aplikasi '{app}' tidak ditemukan"),
            "Jalankan 'skoci status' untuk melihat aplikasi terdaftar atau 'skoci init'",
        )
    }

    pub fn health_check_failed(target: &str, details: &str) -> Self {
        Self::new(
            "health_check_failed",
            format!("Health check gagal untuk {target}: {details}"),
            "Periksa logs aplikasi dengan 'skoci logs' dan pastikan port/path benar",
        )
    }

    pub fn smoke_test_failed(target: &str, details: &str) -> Self {
        Self::new(
            "smoke_test_failed",
            format!("Smoke test gagal untuk {target}: {details}"),
            "Periksa respons endpoint smoke test atau lakukan 'skoci rollback'",
        )
    }

    pub fn docker_error(message: impl Into<String>) -> Self {
        Self::new(
            "docker_error",
            message,
            "Pastikan Docker daemon berjalan dengan 'docker info' atau 'skoci doctor'",
        )
    }
}

impl fmt::Display for SkociError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} (Solusi: {})",
            self.code, self.message, self.fix
        )
    }
}

impl std::error::Error for SkociError {}
