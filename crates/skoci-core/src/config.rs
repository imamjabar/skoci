use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{SkociConfig, MAX_APP_NAME_LEN};
use std::collections::HashSet;
use std::path::Path;

pub const MAX_DOMAINS_PER_ENV: usize = 5;
pub const RESERVED_APP_NAMES: &[&str] = &["skoci", "api", "www", "admin", "dashboard"];

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn parse_config(content: &str) -> SkociResult<SkociConfig> {
        let config: SkociConfig = serde_yaml::from_str(content).map_err(|e| {
            let msg = e.to_string();
            let fix = if msg.starts_with("env.public") {
                "env.public is now a map of NAME: value, for example:\n  env:\n    public:\n      NODE_ENV: production\nRemove legacy lists like '- PORT' (PORT is set automatically from run.port)"
            } else {
                "Check YAML syntax in skoci.yaml (see 'skoci init' in an empty directory for examples)"
            };
            SkociError::config_invalid(format!("Failed to read skoci.yaml: {msg}"), fix)
        })?;
        Self::validate_config(&config)?;
        Ok(config)
    }

    pub fn load_config(path: impl AsRef<Path>) -> SkociResult<SkociConfig> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            SkociError::bad_request(
                format!("skoci.yaml file not found at {}: {e}", path.display()),
                "Run 'skoci init' to generate skoci.yaml",
            )
        })?;
        Self::parse_config(&content)
    }

    pub fn validate_config(cfg: &SkociConfig) -> SkociResult<()> {
        if cfg.version != 1 {
            return Err(SkociError::config_invalid(
                format!("skoci.yaml version '{}' is not supported (must be 1)", cfg.version),
                "Change to 'version: 1' in skoci.yaml",
            ));
        }

        if !is_valid_app_name(&cfg.app) {
            if cfg.app.trim().is_empty() {
                return Err(SkociError::config_invalid(
                    "App name 'app' cannot be empty",
                    "Specify an app name in skoci.yaml (e.g. 'belajar-paper')",
                ));
            }
            if RESERVED_APP_NAMES.contains(&cfg.app.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("App name '{}' is reserved", cfg.app),
                    "Choose an app name not in the reserved list (skoci, api, www, admin, dashboard)",
                ));
            }
            if cfg.app.ends_with("-preview") {
                return Err(SkociError::config_invalid(
                    format!("App name '{}' cannot end with '-preview'", cfg.app),
                    "Remove the '-preview' suffix from the app name",
                ));
            }
            if cfg.app.contains("--") {
                return Err(SkociError::config_invalid(
                    format!("App name '{}' cannot contain consecutive hyphens ('--')", cfg.app),
                    "Replace '--' with a single hyphen '-'",
                ));
            }
            if cfg.app.ends_with('-') {
                return Err(SkociError::config_invalid(
                    format!("App name '{}' cannot end with a hyphen", cfg.app),
                    "Remove the trailing hyphen from the app name",
                ));
            }
            if cfg.app.len() > MAX_APP_NAME_LEN {
                return Err(SkociError::config_invalid(
                    format!(
                        "App name '{}' is too long (maximum {} characters)",
                        cfg.app, MAX_APP_NAME_LEN
                    ),
                    "Use a shorter app name (e.g. 'belajar-paper')",
                ));
            }
            if cfg.app.contains('_') {
                return Err(SkociError::config_invalid(
                    format!(
                        "App name '{}' cannot contain underscores (_)",
                        cfg.app
                    ),
                    "Replace '_' with '-' (e.g. 'belajar-paper')",
                ));
            }
            return Err(SkociError::config_invalid(
                format!(
                    "App name '{}' is invalid. Only lowercase letters, numbers, and '-' are allowed, starting with a letter or number",
                    cfg.app
                ),
                "Use an app name like 'belajar-paper'",
            ));
        }

        if cfg.run.port == 0 {
            return Err(SkociError::config_invalid(
                "App port cannot be 0",
                "Specify a valid port in 'run.port' (e.g. 3000 or 8080)",
            ));
        }

        if !cfg.run.health.starts_with('/') {
            return Err(SkociError::config_invalid(
                format!("Health check path '{}' must start with '/'", cfg.run.health),
                "Change 'run.health' to a path starting with '/', e.g. '/healthz' or '/'",
            ));
        }

        if !(5..=600).contains(&cfg.run.health_timeout_secs) {
            return Err(SkociError::config_invalid(
                format!(
                    "Health check timeout ({}s) must be between 5 and 600 seconds",
                    cfg.run.health_timeout_secs
                ),
                "Change 'run.health_timeout_secs' to a value between 5 and 600 (default: 60)",
            ));
        }

        let mut seen_domains = HashSet::new();
        if cfg.domains.len() > MAX_DOMAINS_PER_ENV {
            return Err(SkociError::config_invalid(
                format!(
                    "Too many domains specified ({} domains, maximum {})",
                    cfg.domains.len(),
                    MAX_DOMAINS_PER_ENV
                ),
                "Reduce the number of domains in 'domains' to 5 or fewer",
            ));
        }
        if cfg.preview_domains.len() > MAX_DOMAINS_PER_ENV {
            return Err(SkociError::config_invalid(
                format!(
                    "Too many preview domains specified ({} domains, maximum {})",
                    cfg.preview_domains.len(),
                    MAX_DOMAINS_PER_ENV
                ),
                "Reduce the number of preview domains in 'preview_domains' to 5 or fewer",
            ));
        }

        for domain in &cfg.domains {
            if !is_valid_hostname(domain) {
                return Err(SkociError::config_invalid(
                    format!("Domain '{domain}' is invalid"),
                    "Use a valid domain name like 'app.example.com' (lowercase, contains dot, not an IP)",
                ));
            }
            if !seen_domains.insert(domain.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Duplicate domain found: '{domain}'"),
                    "Remove duplicate domain from 'domains' or 'preview_domains'",
                ));
            }
        }
        for domain in &cfg.preview_domains {
            if !is_valid_hostname(domain) {
                return Err(SkociError::config_invalid(
                    format!("Preview domain '{domain}' is invalid"),
                    "Use a valid domain name like 'preview.example.com' (lowercase, contains dot, not an IP)",
                ));
            }
            if !seen_domains.insert(domain.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Duplicate domain found: '{domain}'"),
                    "Remove duplicate domain from 'domains' or 'preview_domains'",
                ));
            }
        }

        for (key, val) in &cfg.env.public {
            if !is_valid_env_key(key) || key.contains('\r') || key.contains('\n') || key.contains('\0') {
                return Err(SkociError::config_invalid(
                    format!("Public env name '{key}' is invalid"),
                    "Use uppercase letters and underscores like 'API_URL' or 'NODE_ENV'",
                ));
            }
            if key.starts_with("SKOCI_") {
                return Err(SkociError::config_invalid(
                    format!("Public env name '{key}' cannot start with 'SKOCI_'"),
                    "Names starting with 'SKOCI_' are reserved for the Skoci system",
                ));
            }
            if val.contains('\r') || val.contains('\n') || val.contains('\0') {
                return Err(SkociError::config_invalid(
                    format!("Public env value for '{key}' contains invalid characters (newline or null byte)"),
                    "Remove carriage returns, newlines, and null bytes from env values",
                ));
            }
            if val.len() > 4096 {
                return Err(SkociError::config_invalid(
                    format!(
                        "Public env value for '{key}' exceeds maximum length of 4096 bytes ({} bytes)",
                        val.len()
                    ),
                    "Keep public env values under 4096 bytes",
                ));
            }
        }

        let mut seen_secrets = HashSet::new();
        for secret in &cfg.env.secrets {
            if !is_valid_env_key(secret) {
                return Err(SkociError::config_invalid(
                    format!("Secret name '{secret}' is invalid"),
                    "Use uppercase letters like 'DATABASE_PASSWORD' or 'API_KEY'",
                ));
            }
            if secret.starts_with("SKOCI_") {
                return Err(SkociError::config_invalid(
                    format!("Secret name '{secret}' cannot start with 'SKOCI_'"),
                    "Names starting with 'SKOCI_' are reserved for the Skoci system",
                ));
            }
            if !seen_secrets.insert(secret.as_str()) {
                return Err(SkociError::config_invalid(
                    format!("Duplicate secret name: '{secret}'"),
                    "Remove duplicate secret name from 'env.secrets'",
                ));
            }
        }

        for (name, svc) in &cfg.services {
            if !is_valid_service_name(name) {
                return Err(SkociError::config_invalid(
                    format!(
                        "Service name '{name}' is invalid (must be 1-20 characters, start with lowercase letter, only lowercase letters and numbers, no hyphens)",
                    ),
                    "Use a service name like 'db', 'cache', or 'queue1'",
                ));
            }
            if svc.version.trim().is_empty() {
                return Err(SkociError::config_invalid(
                    format!("Service version for '{name}' cannot be empty"),
                    "Specify a service version (e.g. '16' for postgres or '8' for valkey)",
                ));
            }
            if let Some(mem) = &svc.memory {
                if !is_valid_memory_spec(mem) {
                    return Err(SkociError::config_invalid(
                        format!("Memory value for service '{name}' ('{mem}') is invalid"),
                        "Use a number followed by b, k, m, or g (e.g. '256m' or '1g')",
                    ));
                }
            }
            if let Some(backup) = &svc.backup {
                if !(1..=30).contains(&backup.keep) {
                    return Err(SkociError::config_invalid(
                        format!(
                            "Backup keep count '{}' for service '{name}' is invalid (must be between 1 and 30)",
                            backup.keep
                        ),
                        "Specify a keep count between 1 and 30 (e.g. 'keep: 7')",
                    ));
                }
                let fields: Vec<&str> = backup.schedule.split_whitespace().collect();
                if fields.len() != 5 {
                    return Err(SkociError::config_invalid(
                        format!(
                            "Backup schedule '{}' for service '{name}' is invalid (must have 5 fields)",
                            backup.schedule
                        ),
                        "Use 5-field cron format: minute hour day_of_month month day_of_week (e.g. '0 2 * * *')",
                    ));
                }
                let min: u32 = fields[0].parse().map_err(|_| {
                    SkociError::config_invalid(
                        format!(
                            "Backup schedule minute '{}' for service '{name}' must be a single integer 0-59 to ensure at most hourly backups",
                            fields[0]
                        ),
                        "Specify an exact minute number like '0' in '0 2 * * *'",
                    )
                })?;
                if min > 59 {
                    return Err(SkociError::config_invalid(
                        format!(
                            "Backup schedule minute '{}' for service '{name}' is out of range (must be 0-59)",
                            min
                        ),
                        "Specify a minute between 0 and 59",
                    ));
                }
            }
        }

        if !is_safe_relative_path(&cfg.build.dockerfile) {
            return Err(SkociError::config_invalid(
                format!("Dockerfile path '{}' is unsafe or invalid", cfg.build.dockerfile),
                "Use a relative path without '..' or an absolute path (e.g. 'Dockerfile' or 'docker/Dockerfile')",
            ));
        }

        if !is_safe_relative_path(&cfg.build.context) {
            return Err(SkociError::config_invalid(
                format!("Build context path '{}' is unsafe or invalid", cfg.build.context),
                "Use a relative path without '..' or an absolute path (e.g. '.' or 'frontend')",
            ));
        }

        if let Some(mem) = &cfg.run.memory {
            if !is_valid_memory_spec(mem) {
                return Err(SkociError::config_invalid(
                    format!("Memory value '{mem}' is invalid"),
                    "Use a number followed by b, k, m, or g (e.g. '256m' or '1g')",
                ));
            }
        }

        if let Some(cpus) = cfg.run.cpus {
            if cpus <= 0.0 || cpus > 64.0 || cpus.is_nan() {
                return Err(SkociError::config_invalid(
                    format!("run.cpus value '{cpus}' is invalid"),
                    "cpus value must be greater than 0 and at most 64 (e.g. 0.5 or 2)",
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
    if RESERVED_APP_NAMES.contains(&name) {
        return false;
    }
    if name.contains("--") || name.ends_with('-') || name.ends_with("-preview") {
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

pub fn is_valid_service_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 20 {
        return false;
    }
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c.is_ascii_lowercase(),
        None => false,
    };
    first && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
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
