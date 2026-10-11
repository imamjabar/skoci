use chrono::Utc;
use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::normalize_spki_pin;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerEntry {
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub pin: Option<String>,
    pub created_at: String,
}

impl std::fmt::Debug for ServerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerEntry")
            .field("url", &self.url)
            .field("token", &"[REDACTED]")
            .field("pin", &self.pin)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProfileConfig {
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub servers: HashMap<String, ServerEntry>,
}

impl ProfileConfig {
    pub fn get(&self, name: &str) -> Option<&ServerEntry> {
        self.servers.get(name)
    }

    pub fn upsert(&mut self, name: &str, url: &str, token: &str, pin: Option<&str>) -> SkociResult<()> {
        let normalized_pin = match pin {
            Some(p) if !p.trim().is_empty() => {
                match normalize_spki_pin(p) {
                    Some(np) => Some(np),
                    None => {
                        return Err(SkociError::bad_request(
                            format!("Malformed SPKI pin '{p}': expected sha256:<64 hex> or bare 64-char hex"),
                            "Check the pin format or retrieve a fresh pin with 'skoci server fingerprint'",
                        ));
                    }
                }
            }
            _ => None,
        };

        let entry = ServerEntry {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            pin: normalized_pin,
            created_at: Utc::now().to_rfc3339(),
        };

        self.servers.insert(name.to_string(), entry);
        if self.current.is_none() {
            self.current = Some(name.to_string());
        }
        Ok(())
    }

    pub fn set_current(&mut self, name: &str) -> SkociResult<()> {
        if !self.servers.contains_key(name) {
            return Err(SkociError::not_found(
                format!("Server profile '{name}' not found"),
                "Add the profile first or check available profiles",
            ));
        }
        self.current = Some(name.to_string());
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let existed = self.servers.remove(name).is_some();
        if self.current.as_deref() == Some(name) {
            self.current = self.servers.keys().next().cloned();
        }
        existed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSource {
    Flag,
    ProjectEnv,
    Profile(String),
}

impl std::fmt::Display for TargetSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetSource::Flag => write!(f, "flag"),
            TargetSource::ProjectEnv => write!(f, "project"),
            TargetSource::Profile(name) => write!(f, "profile:{name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget {
    pub url: String,
    pub token: Option<String>,
    pub pin: Option<String>,
    pub source: TargetSource,
}

pub fn default_profile_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("skoci").join("servers.json");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join(".config").join("skoci").join("servers.json");
        }
    }
    PathBuf::from(".config").join("skoci").join("servers.json")
}

pub fn load_profiles() -> SkociResult<ProfileConfig> {
    load_profiles_from(&default_profile_path())
}

pub fn load_profiles_from(path: &Path) -> SkociResult<ProfileConfig> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(SkociError::bad_request(
                    format!("Refusing to read profile config: {} is a symlink", path.display()),
                    "Use a regular file instead of a symlink for security",
                ));
            }
            if !meta.file_type().is_file() {
                return Err(SkociError::bad_request(
                    format!("Refusing to read profile config: {} is not a regular file", path.display()),
                    "Ensure servers.json is a regular file",
                ));
            }
            let data = std::fs::read_to_string(path).map_err(|e| {
                SkociError::internal(
                    format!("Failed to read profile config at {}: {e}", path.display()),
                    "Check file access permissions",
                )
            })?;
            serde_json::from_str::<ProfileConfig>(&data).map_err(|e| {
                SkociError::bad_request(
                    format!("Failed to parse profile config at {}: {e}", path.display()),
                    "Check JSON formatting in servers.json",
                )
            })
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(ProfileConfig::default()),
        Err(err) => Err(SkociError::internal(
            format!("Failed to access profile config at {}: {e}", path.display(), e = err),
            "Check directory permissions",
        )),
    }
}

pub fn save_profiles(config: &ProfileConfig) -> SkociResult<()> {
    save_profiles_to(&default_profile_path(), config)
}

pub fn save_profiles_to(path: &Path, config: &ProfileConfig) -> SkociResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SkociError::internal(
            format!("Invalid profile path: {}", path.display()),
            "Check configuration path",
        )
    })?;

    if !parent.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(parent).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create config directory {}: {e}", parent.display()),
                    "Check filesystem permissions",
                )
            })?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create config directory {}: {e}", parent.display()),
                    "Check filesystem permissions",
                )
            })?;
        }
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    let tmp_path = parent.join(format!(".servers.json.tmp.{}", uuid::Uuid::new_v4()));

    let res = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }

        let json_bytes = serde_json::to_vec_pretty(config).map_err(std::io::Error::other)?;
        file.write_all(&json_bytes)?;
        file.sync_all()?;
        drop(file);

        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if let Err(err) = res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SkociError::internal(
            format!("Failed to save profile config at {}: {err}", path.display()),
            "Check filesystem permissions and disk space",
        ));
    }

    Ok(())
}

pub fn get_profile(name: &str) -> SkociResult<Option<ServerEntry>> {
    let config = load_profiles()?;
    Ok(config.servers.get(name).cloned())
}

pub fn get_current_profile() -> SkociResult<Option<(String, ServerEntry)>> {
    let config = load_profiles()?;
    Ok(config
        .current
        .as_ref()
        .and_then(|c| config.servers.get(c).map(|e| (c.clone(), e.clone()))))
}

pub fn upsert_profile(name: &str, url: &str, token: &str, pin: Option<&str>) -> SkociResult<()> {
    let mut config = load_profiles()?;
    config.upsert(name, url, token, pin)?;
    save_profiles(&config)
}

pub fn save_server(
    name: &str,
    url: &str,
    token: &str,
    pin: Option<&str>,
    set_current: bool,
) -> SkociResult<()> {
    let mut config = load_profiles()?;
    config.upsert(name, url, token, pin)?;
    if set_current {
        config.current = Some(name.to_string());
    }
    save_profiles(&config)
}

pub fn set_current_profile(name: &str) -> SkociResult<()> {
    let mut config = load_profiles()?;
    config.set_current(name)?;
    save_profiles(&config)
}

pub fn remove_profile(name: &str) -> SkociResult<bool> {
    let mut config = load_profiles()?;
    let removed = config.remove(name);
    save_profiles(&config)?;
    Ok(removed)
}

pub fn resolve_target(
    explicit_url: Option<&str>,
    explicit_token: Option<String>,
    explicit_pin: Option<&str>,
    profile_name: Option<&str>,
    project_env: Option<(String, String, Option<String>)>,
) -> SkociResult<ResolvedTarget> {
    let profiles = load_profiles()?;
    resolve_target_with_profiles(
        explicit_url,
        explicit_token,
        explicit_pin,
        profile_name,
        project_env,
        &profiles,
    )
}

pub fn resolve_target_with_profiles(
    explicit_url: Option<&str>,
    explicit_token: Option<String>,
    explicit_pin: Option<&str>,
    profile_name: Option<&str>,
    project_env: Option<(String, String, Option<String>)>,
    profiles: &ProfileConfig,
) -> SkociResult<ResolvedTarget> {
    // 1. Explicit flags: explicit_url wins
    if let Some(url) = explicit_url {
        let pin = explicit_pin.map(|s| s.to_string());
        return Ok(ResolvedTarget {
            url: url.trim_end_matches('/').to_string(),
            token: explicit_token,
            pin,
            source: TargetSource::Flag,
        });
    }

    // 2. Project environment (.env.skoci)
    if let Some((env_url, env_token, env_pin)) = project_env {
        let token = explicit_token.or(Some(env_token));
        let pin = explicit_pin.map(|s| s.to_string()).or(env_pin);
        return Ok(ResolvedTarget {
            url: env_url.trim_end_matches('/').to_string(),
            token,
            pin,
            source: TargetSource::ProjectEnv,
        });
    }

    // 3. Named profile
    if let Some(name) = profile_name {
        let entry = profiles.servers.get(name).ok_or_else(|| {
            SkociError::not_found(
                format!("Server profile '{name}' not found"),
                "Run 'skoci server setup' or list profiles to see available configurations",
            )
        })?;
        let token = explicit_token.or_else(|| Some(entry.token.clone()));
        let pin = explicit_pin.map(|s| s.to_string()).or_else(|| entry.pin.clone());
        return Ok(ResolvedTarget {
            url: entry.url.clone(),
            token,
            pin,
            source: TargetSource::Profile(name.to_string()),
        });
    }

    // 4. Current profile
    if let Some(current_name) = &profiles.current {
        if let Some(entry) = profiles.servers.get(current_name) {
            let token = explicit_token.or_else(|| Some(entry.token.clone()));
            let pin = explicit_pin.map(|s| s.to_string()).or_else(|| entry.pin.clone());
            return Ok(ResolvedTarget {
                url: entry.url.clone(),
                token,
                pin,
                source: TargetSource::Profile(current_name.clone()),
            });
        }
    }

    Err(SkociError::bad_request(
        "No target Skoci server configured",
        "Pass --server <url>, configure .env.skoci, or select a profile with 'skoci profile switch <name>'",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_round_trip() {
        let tmp = std::env::temp_dir().join(format!("test-profiles-{}.json", uuid::Uuid::new_v4()));
        let mut cfg = ProfileConfig::default();
        cfg.upsert("prod", "https://prod.example.com", "tok1", Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")).unwrap();
        cfg.upsert("staging", "https://staging.example.com", "tok2", None).unwrap();
        cfg.set_current("staging").unwrap();

        save_profiles_to(&tmp, &cfg).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&tmp).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        let loaded = load_profiles_from(&tmp).unwrap();
        assert_eq!(loaded.current.as_deref(), Some("staging"));
        let prod = loaded.get("prod").unwrap();
        assert_eq!(prod.url, "https://prod.example.com");
        assert_eq!(prod.token, "tok1");
        assert_eq!(
            prod.pin.as_deref(),
            Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_symlink_refused() {
        let dir = std::env::temp_dir().join(format!("test-symlink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let real_file = dir.join("real.json");
        std::fs::write(&real_file, "{}").unwrap();
        let link_file = dir.join("link.json");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &link_file).unwrap();
            let res = load_profiles_from(&link_file);
            assert!(res.is_err());
            let err = res.unwrap_err();
            assert!(err.message.contains("symlink"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_target_precedence() {
        let mut cfg = ProfileConfig::default();
        cfg.upsert("myprof", "https://prof.skoci.com", "proftok", Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")).unwrap();
        cfg.upsert("currentprof", "https://current.skoci.com", "currtok", None).unwrap();
        cfg.set_current("currentprof").unwrap();

        // 1. Explicit url beats everything
        let res = resolve_target_with_profiles(
            Some("https://flag.skoci.com"),
            Some("flagtok".to_string()),
            Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Some("myprof"),
            Some(("https://env.skoci.com".to_string(), "envtok".to_string(), None)),
            &cfg,
        ).unwrap();
        assert_eq!(res.source, TargetSource::Flag);
        assert_eq!(res.source.to_string(), "flag");
        assert_eq!(res.url, "https://flag.skoci.com");
        assert_eq!(res.token.as_deref(), Some("flagtok"));
        assert_eq!(res.pin.as_deref(), Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));

        // 2. Project env beats named and current profile
        let res = resolve_target_with_profiles(
            None,
            None,
            None,
            Some("myprof"),
            Some(("https://env.skoci.com".to_string(), "envtok".to_string(), Some("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()))),
            &cfg,
        ).unwrap();
        assert_eq!(res.source, TargetSource::ProjectEnv);
        assert_eq!(res.source.to_string(), "project");
        assert_eq!(res.url, "https://env.skoci.com");
        assert_eq!(res.token.as_deref(), Some("envtok"));
        assert_eq!(res.pin.as_deref(), Some("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));

        // 3. Named profile beats current profile
        let res = resolve_target_with_profiles(
            None,
            None,
            None,
            Some("myprof"),
            None,
            &cfg,
        ).unwrap();
        assert_eq!(res.source, TargetSource::Profile("myprof".to_string()));
        assert_eq!(res.source.to_string(), "profile:myprof");
        assert_eq!(res.url, "https://prof.skoci.com");
        assert_eq!(res.token.as_deref(), Some("proftok"));
        assert_eq!(res.pin.as_deref(), Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));

        // 4. Current profile wins when nothing else specified
        let res = resolve_target_with_profiles(
            None,
            None,
            None,
            None,
            None,
            &cfg,
        ).unwrap();
        assert_eq!(res.source, TargetSource::Profile("currentprof".to_string()));
        assert_eq!(res.source.to_string(), "profile:currentprof");
        assert_eq!(res.url, "https://current.skoci.com");
        assert_eq!(res.token.as_deref(), Some("currtok"));
        assert_eq!(res.pin, None);

        // 5. Empty config errors
        let empty_cfg = ProfileConfig::default();
        let res = resolve_target_with_profiles(None, None, None, None, None, &empty_cfg);
        assert!(res.is_err());
    }
}
