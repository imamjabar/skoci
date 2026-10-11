use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{SkociConfig, SkociEvidence, SkociPolicy};
use std::path::Path;

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn parse_config(content: &str) -> SkociResult<SkociConfig> {
        let config: SkociConfig = serde_yaml::from_str(content)
            .map_err(|e| SkociError::config_invalid(
                format!("Gagal membaca skoci.yaml: {e}"),
                "Periksa sintaks YAML pada file skoci.yaml",
            ))?;
        Self::validate_config(&config)?;
        Ok(config)
    }

    pub fn load_config(path: impl AsRef<Path>) -> SkociResult<SkociConfig> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            SkociError::bad_request(
                format!("File skoci.yaml tidak ditemukan di {}: {e}", path.display()),
                "Jalankan 'skoci init' untuk membuat skoci.yaml",
            )
        })?;
        Self::parse_config(&content)
    }

    pub fn validate_config(config: &SkociConfig) -> SkociResult<()> {
        if config.version != 1 {
            return Err(SkociError::config_invalid(
                format!("Versi skoci.yaml '{}' tidak didukung (harus 1)", config.version),
                "Ganti 'version: 1' pada skoci.yaml",
            ));
        }

        if config.app.trim().is_empty() {
            return Err(SkociError::config_invalid(
                "Nama aplikasi 'app' tidak boleh kosong",
                "Tentukan nama aplikasi pada skoci.yaml",
            ));
        }

        // Enforce strict character set to prevent injection: [a-z0-9_-]
        if !config.app.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            return Err(SkociError::config_invalid(
                format!("Nama aplikasi '{}' tidak valid. Hanya boleh huruf kecil, angka, '-' dan '_'", config.app),
                "Gunakan karakter aman seperti 'my-app' atau 'toko_v1'",
            ));
        }

        if config.run.port == 0 {
            return Err(SkociError::config_invalid(
                "Port aplikasi tidak boleh 0",
                "Tentukan port yang valid pada 'run.port' (misal 3000)",
            ));
        }

        // Validate domain names against shell injection
        for domain in &config.domains {
            if domain.contains(' ') || domain.contains(';') || domain.contains('&') || domain.contains('$') {
                return Err(SkociError::config_invalid(
                    format!("Domain '{domain}' mengandung karakter tidak valid"),
                    "Gunakan nama domain valid seperti 'api.example.com'",
                ));
            }
        }

        Ok(())
    }

    pub fn parse_policy(content: &str) -> SkociResult<SkociPolicy> {
        serde_yaml::from_str(content).map_err(|e| {
            SkociError::config_invalid(
                format!("Gagal membaca skoci.policy.yaml: {e}"),
                "Periksa sintaks YAML pada file skoci.policy.yaml",
            )
        })
    }

    pub fn load_policy(path: impl AsRef<Path>) -> SkociResult<SkociPolicy> {
        let path = path.as_ref();
        if !path.exists() {
            // Return conservative default policy if missing
            return Ok(SkociPolicy {
                version: 1,
                preview: Default::default(),
                production: Default::default(),
            });
        }
        let content = std::fs::read_to_string(path).map_err(|e| {
            SkociError::bad_request(
                format!("Gagal membaca skoci.policy.yaml: {e}"),
                "Pastikan file skoci.policy.yaml dapat dibaca",
            )
        })?;
        Self::parse_policy(&content)
    }

    pub fn parse_evidence(content: &str) -> SkociResult<SkociEvidence> {
        serde_json::from_str(content).map_err(|e| {
            SkociError::bad_request(
                format!("Gagal membaca skoci.evidence.json: {e}"),
                "Pastikan skoci.evidence.json valid JSON",
            )
        })
    }

    pub fn save_evidence(path: impl AsRef<Path>, evidence: &SkociEvidence) -> SkociResult<()> {
        let content = serde_json::to_string_pretty(evidence).map_err(|e| {
            SkociError::bad_request(format!("Gagal serialisasi evidence: {e}"), "Coba deploy ulang")
        })?;
        std::fs::write(path, content).map_err(|e| {
            SkociError::bad_request(
                format!("Gagal menyimpan skoci.evidence.json: {e}"),
                "Periksa izin tulis di direktori proyek",
            )
        })?;
        Ok(())
    }
}
