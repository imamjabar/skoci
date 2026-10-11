use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{SkociConfig, MAX_APP_NAME_LEN};
use std::collections::HashSet;
use std::path::Path;

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn parse_config(content: &str) -> SkociResult<SkociConfig> {
        let config: SkociConfig = serde_yaml::from_str(content).map_err(|e| {
            let msg = e.to_string();
            let fix = if msg.starts_with("env.public") {
                "env.public sekarang berupa pasangan NAMA: nilai, contoh:\n  env:\n    public:\n      NODE_ENV: production\nHapus daftar lama seperti '- PORT' (PORT diisi otomatis dari run.port)"
            } else {
                "Periksa sintaks YAML pada file skoci.yaml (lihat contoh di 'skoci init' pada folder kosong)"
            };
            SkociError::config_invalid(format!("Gagal membaca skoci.yaml: {msg}"), fix)
        })?;
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

    pub fn validate_config(cfg: &SkociConfig) -> SkociResult<()> {
        if cfg.version != 1 {
            return Err(SkociError::config_invalid(
                format!("Versi skoci.yaml '{}' tidak didukung (harus 1)", cfg.version),
                "Ganti 'version: 1' pada skoci.yaml",
            ));
        }

        if !is_valid_app_name(&cfg.app) {
            if cfg.app.trim().is_empty() {
                return Err(SkociError::config_invalid(
                    "Nama aplikasi 'app' tidak boleh kosong",
                    "Tentukan nama aplikasi pada skoci.yaml (contoh: 'belajar-paper')",
                ));
            }
            if cfg.app.len() > MAX_APP_NAME_LEN {
                return Err(SkociError::config_invalid(
                    format!(
                        "Nama aplikasi '{}' terlalu panjang (maksimal {} karakter)",
                        cfg.app, MAX_APP_NAME_LEN
                    ),
                    "Gunakan nama aplikasi yang lebih pendek (contoh: 'belajar-paper')",
                ));
            }
            if cfg.app.contains('_') {
                return Err(SkociError::config_invalid(
                    format!(
                        "Nama aplikasi '{}' tidak boleh menggunakan garis bawah (_)",
                        cfg.app
                    ),
                    "Ganti '_' dengan '-' (contoh: 'belajar-paper')",
                ));
            }
            return Err(SkociError::config_invalid(
                format!(
                    "Nama aplikasi '{}' tidak valid. Hanya boleh huruf kecil, angka, dan '-' serta diawali huruf kecil atau angka",
                    cfg.app
                ),
                "Gunakan nama aplikasi seperti 'belajar-paper'",
            ));
        }

        if cfg.run.port == 0 {
            return Err(SkociError::config_invalid(
                "Port aplikasi tidak boleh 0",
                "Tentukan port yang valid pada 'run.port' (misal: 3000 atau 8080)",
            ));
        }

        if !cfg.run.health.starts_with('/') {
            return Err(SkociError::config_invalid(
                format!("Path health check '{}' harus diawali dengan '/'", cfg.run.health),
                "Ubah 'run.health' menjadi path yang diawali '/', misal '/healthz' atau '/'",
            ));
        }

        if !(5..=600).contains(&cfg.run.health_timeout_secs) {
            return Err(SkociError::config_invalid(
                format!(
                    "Timeout health check ({} detik) harus antara 5 hingga 600 detik",
                    cfg.run.health_timeout_secs
                ),
                "Ubah 'run.health_timeout_secs' menjadi nilai antara 5 dan 600 (default: 60)",
            ));
        }

        let mut seen_domains = HashSet::new();
        for domain in &cfg.domains {
            if !is_valid_hostname(domain) {
                return Err(SkociError::config_invalid(
                    format!("Domain '{domain}' tidak valid"),
                    "Gunakan nama domain valid seperti 'app.contoh.com' (huruf kecil, ada titik, bukan IP)",
                ));
            }
            if !seen_domains.insert(domain.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Domain duplikat ditemukan: '{domain}'"),
                    "Hapus domain yang duplikat dari 'domains' atau 'preview_domains'",
                ));
            }
        }
        for domain in &cfg.preview_domains {
            if !is_valid_hostname(domain) {
                return Err(SkociError::config_invalid(
                    format!("Preview domain '{domain}' tidak valid"),
                    "Gunakan nama domain valid seperti 'preview.contoh.com' (huruf kecil, ada titik, bukan IP)",
                ));
            }
            if !seen_domains.insert(domain.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Domain duplikat ditemukan: '{domain}'"),
                    "Hapus domain yang duplikat dari 'domains' atau 'preview_domains'",
                ));
            }
        }

        for key in cfg.env.public.keys() {
            if !is_valid_env_key(key) {
                return Err(SkociError::config_invalid(
                    format!("Nama env publik '{key}' tidak valid"),
                    "Gunakan format huruf kapital seperti 'API_URL' atau 'NODE_ENV'",
                ));
            }
            if key.starts_with("SKOCI_") {
                return Err(SkociError::config_invalid(
                    format!("Nama env publik '{key}' tidak boleh diawali dengan 'SKOCI_'"),
                    "Nama dengan awalan 'SKOCI_' dikhususkan untuk sistem Skoci",
                ));
            }
        }

        let mut seen_secrets = HashSet::new();
        for secret in &cfg.env.secrets {
            if !is_valid_env_key(secret) {
                return Err(SkociError::config_invalid(
                    format!("Nama secret '{secret}' tidak valid"),
                    "Gunakan format huruf kapital seperti 'DATABASE_PASSWORD' atau 'API_KEY'",
                ));
            }
            if secret.starts_with("SKOCI_") {
                return Err(SkociError::config_invalid(
                    format!("Nama secret '{secret}' tidak boleh diawali dengan 'SKOCI_'"),
                    "Nama dengan awalan 'SKOCI_' dikhususkan untuk sistem Skoci",
                ));
            }
            if !seen_secrets.insert(secret.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Nama secret duplikat: '{secret}'"),
                    "Hapus nama secret yang duplikat dari 'env.secrets'",
                ));
            }
        }

        for (name, svc) in &cfg.services {
            if !is_valid_service_name(name) {
                return Err(SkociError::config_invalid(
                    format!(
                        "Nama service '{name}' tidak valid (harus 1-20 karakter, diawali huruf kecil, hanya huruf kecil, angka, dan '-')",
                    ),
                    "Gunakan nama service seperti 'db', 'cache', atau 'queue'",
                ));
            }
            if svc.version.trim().is_empty() {
                return Err(SkociError::config_invalid(
                    format!("Versi service '{name}' tidak boleh kosong"),
                    "Tentukan versi service (misal: '16' untuk postgres atau '8' untuk valkey)",
                ));
            }
            if let Some(mem) = &svc.memory {
                if !is_valid_memory_spec(mem) {
                    return Err(SkociError::config_invalid(
                        format!("Nilai memory service '{name}' ('{mem}') tidak valid"),
                        "Gunakan format angka diikuti satuan b, k, m, atau g (contoh: '256m' atau '1g')",
                    ));
                }
            }
        }

        if !is_safe_relative_path(&cfg.build.dockerfile) {
            return Err(SkociError::config_invalid(
                format!("Path Dockerfile '{}' tidak aman atau tidak valid", cfg.build.dockerfile),
                "Gunakan path relatif tanpa '..' atau path absolut (misal: 'Dockerfile' atau 'docker/Dockerfile')",
            ));
        }

        if !is_safe_relative_path(&cfg.build.context) {
            return Err(SkociError::config_invalid(
                format!("Path build context '{}' tidak aman atau tidak valid", cfg.build.context),
                "Gunakan path relatif tanpa '..' atau path absolut (misal: '.' atau 'frontend')",
            ));
        }

        if let Some(mem) = &cfg.run.memory {
            if !is_valid_memory_spec(mem) {
                return Err(SkociError::config_invalid(
                    format!("Nilai memory '{mem}' tidak valid"),
                    "Gunakan format angka diikuti satuan b, k, m, atau g (contoh: '256m' atau '1g')",
                ));
            }
        }

        if let Some(cpus) = cfg.run.cpus {
            if cpus <= 0.0 || cpus > 64.0 || cpus.is_nan() {
                return Err(SkociError::config_invalid(
                    format!("Nilai run.cpus '{cpus}' tidak valid"),
                    "Nilai cpus harus lebih besar dari 0 dan maksimal 64 (contoh: 0.5 atau 2)",
                ));
            }
        }

        Ok(())
    }
}

pub fn is_valid_app_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_APP_NAME_LEN {
        return false;
    }
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c.is_ascii_lowercase() || c.is_ascii_digit(),
        None => false,
    };
    first && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn is_valid_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if !host.contains('.') {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    for label in &labels {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if !label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return false;
        }
    }
    if let Some(last) = labels.last() {
        if last.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

pub fn is_valid_env_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let first = match chars.next() {
        Some(c) => c.is_ascii_uppercase() || c == '_',
        None => false,
    };
    first && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub fn is_safe_relative_path(p: &str) -> bool {
    let p = p.trim();
    if p.is_empty() {
        return false;
    }
    let path = Path::new(p);
    if path.is_absolute() {
        return false;
    }
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => return false,
            _ => {}
        }
    }
    true
}

fn is_valid_service_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 20 {
        return false;
    }
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c.is_ascii_lowercase(),
        None => false,
    };
    first && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_valid_memory_spec(mem: &str) -> bool {
    if mem.len() < 2 {
        return false;
    }
    let last = mem.chars().last().unwrap();
    if !matches!(last, 'b' | 'k' | 'm' | 'g') {
        return false;
    }
    let num_str = &mem[..mem.len() - 1];
    !num_str.is_empty() && num_str.chars().all(|c| c.is_ascii_digit())
}
