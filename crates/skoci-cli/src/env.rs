use std::path::Path;
use skoci_api::models::{LEGACY_ENV_PATH, PROJECT_DIR, PROJECT_ENV_PATH};
use skoci_api::{SkociError, SkociResult};
pub fn parse_env_file_content(content: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let line = if let Some(stripped) = trimmed.strip_prefix("export ") {
            stripped.trim()
        } else {
            trimmed
        };

        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim();
            let val = if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                if val.len() >= 2 {
                    &val[1..val.len() - 1]
                } else {
                    val
                }
            } else {
                val
            };
            pairs.push((key, val.to_string()));
        }
    }
    pairs
}

pub fn is_local_url(url_str: &str) -> bool {
    if let Ok(parsed) = reqwest::Url::parse(url_str) {
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return false;
        }
        if let Some(host) = parsed.host_str() {
            let h = host.trim_start_matches('[').trim_end_matches(']');
            h.eq_ignore_ascii_case("127.0.0.1")
                || h.eq_ignore_ascii_case("localhost")
                || h == "::1"
        } else {
            false
        }
    } else {
        false
    }
}

pub fn filter_skoci_env_pairs_with_source(pairs: Vec<(String, String)>, source: &str) -> Vec<(String, String)> {
    let mut filtered = Vec::new();
    for (k, v) in pairs {
        if k == "SKOCI_SERVER_URL" || k == "SKOCI_TOKEN" || k == "SKOCI_SERVER_PIN" {
            filtered.push((k, v));
        } else {
            eprintln!("Warning: Ignoring disallowed key '{k}' in {source} (only SKOCI_SERVER_URL, SKOCI_TOKEN, and SKOCI_SERVER_PIN are allowed)");
        }
    }
    filtered
}

pub fn filter_skoci_env_pairs(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    filter_skoci_env_pairs_with_source(pairs, PROJECT_ENV_PATH)
}

pub fn load_env_files() {
    load_env_files_at(Path::new("."));
}

pub fn load_env_files_at(root: &Path) {
    let modern = root.join(PROJECT_ENV_PATH);
    let target = if modern.exists() {
        Some(modern)
    } else {
        let legacy = root.join(LEGACY_ENV_PATH);
        if legacy.exists() {
            Some(legacy)
        } else {
            None
        }
    };

    if let Some(path) = target {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let pairs = filter_skoci_env_pairs_with_source(parse_env_file_content(&content), &path.display().to_string());
            for (k, v) in pairs {
                if std::env::var(&k).is_err() {
                    std::env::set_var(&k, &v);
                }
            }
        }
    }
}

pub fn local_master_token() -> Option<String> {
    local_master_token_from_path(Path::new("/etc/skoci/skoci.env"))
}

pub fn local_master_token_from_path(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            for (k, v) in parse_env_file_content(&content) {
                if k == "SKOCI_MASTER_TOKEN" && !v.trim().is_empty() {
                    return Some(v.trim().to_string());
                }
            }
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Permission denied reading {}: {e}", path.display());
            eprintln!("Fix: sudo usermod -aG skoci $USER then log in again, or use sudo.");
            None
        }
        Err(_) => None,
    }
}

pub fn read_project_env() -> Option<(String, String, Option<String>)> {
    read_project_env_at(Path::new("."))
}

pub fn read_project_env_at(root: &Path) -> Option<(String, String, Option<String>)> {
    let modern = root.join(PROJECT_ENV_PATH);
    if modern.exists() {
        return read_project_env_from_path(&modern);
    }
    let legacy = root.join(LEGACY_ENV_PATH);
    if legacy.exists() {
        return read_project_env_from_path(&legacy);
    }
    None
}

pub fn read_project_env_from_path(path: &Path) -> Option<(String, String, Option<String>)> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let pairs = filter_skoci_env_pairs_with_source(parse_env_file_content(&content), &path.display().to_string());
    let mut url = None;
    let mut token = None;
    let mut pin = None;
    for (k, v) in pairs {
        match k.as_str() {
            "SKOCI_SERVER_URL" => url = Some(v),
            "SKOCI_TOKEN" => token = Some(v),
            "SKOCI_SERVER_PIN" => pin = Some(v),
            _ => {}
        }
    }
    match (url, token) {
        (Some(u), Some(t)) => Some((u, t, pin)),
        _ => None,
    }
}

pub fn write_project_env(server_url: &str, token: &str, pin: Option<&str>) -> SkociResult<()> {
    write_project_env_at(Path::new("."), server_url, token, pin)
}

pub fn write_project_env_at(root: &Path, server_url: &str, token: &str, pin: Option<&str>) -> SkociResult<()> {
    let project_dir = root.join(PROJECT_DIR);
    if !project_dir.exists() {
        std::fs::create_dir_all(&project_dir).map_err(|e| {
            SkociError::internal(
                format!("Failed to create {} directory: {e}", project_dir.display()),
                "Check directory write permissions",
            )
        })?;
    }

    let target_path = root.join(PROJECT_ENV_PATH);
    if target_path.exists() || target_path.is_symlink() {
        if let Ok(meta) = std::fs::symlink_metadata(&target_path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(SkociError::bad_request(
                    format!("{} is a symlink or special file", target_path.display()),
                    format!("Remove the symlink before connecting: rm {}", target_path.display()),
                ));
            }
        }
    }

    let mut env_content = format!("SKOCI_SERVER_URL={server_url}\nSKOCI_TOKEN={token}\n");
    if let Some(p) = pin {
        env_content.push_str(&format!("SKOCI_SERVER_PIN={p}\n"));
    }

    let tmp_path = project_dir.join(format!(".env.tmp.{}", uuid::Uuid::new_v4().simple()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut tmp_file = opts.open(&tmp_path).map_err(|e| {
        SkociError::internal(
            format!("Failed to create temporary config file: {e}"),
            "Check write permissions in .skoci directory",
        )
    })?;
    use std::io::Write;
    if let Err(e) = tmp_file.write_all(env_content.as_bytes()) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SkociError::internal(format!("Failed to write config: {e}"), "Retry"));
    }
    drop(tmp_file);
    if let Err(e) = std::fs::rename(&tmp_path, &target_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SkociError::internal(format!("Failed to save {}: {e}", target_path.display()), "Retry"));
    }

    Ok(())
}

pub fn ensure_gitignore_ignores_env(root: &Path) -> SkociResult<()> {
    let gitignore_file = root.join(".gitignore");
    if gitignore_file.exists() {
        let content = std::fs::read_to_string(&gitignore_file).unwrap_or_default();
        if !content.lines().any(|l| l.trim() == PROJECT_ENV_PATH) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&gitignore_file)
                .map_err(|e| {
                    SkociError::internal(
                        format!("Failed to update .gitignore: {e}"),
                        "Check write permissions for .gitignore",
                    )
                })?;
            let suffix = if content.is_empty() || content.ends_with('\n') {
                format!("{PROJECT_ENV_PATH}\n")
            } else {
                format!("\n{PROJECT_ENV_PATH}\n")
            };
            f.write_all(suffix.as_bytes()).map_err(|e| {
                SkociError::internal(
                    format!("Failed to update .gitignore: {e}"),
                    "Check write permissions for .gitignore",
                )
            })?;
        }
    } else {
        std::fs::write(&gitignore_file, format!("{PROJECT_ENV_PATH}\n")).map_err(|e| {
            SkociError::internal(
                format!("Failed to create .gitignore: {e}"),
                "Check write permissions for .gitignore",
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_file_content() {
        let sample = r#"
# This is a comment
SKOCI_SERVER_URL=http://localhost:8787
SKOCI_TOKEN="sk_test_token_123"
OTHER_VAL='single-quoted'
export EXPORTED=val123

EMPTY_LINE=
SPACED = spaced_val
"#;
        let pairs = parse_env_file_content(sample);
        assert_eq!(pairs.len(), 6);
        assert_eq!(pairs[0], ("SKOCI_SERVER_URL".to_string(), "http://localhost:8787".to_string()));
        assert_eq!(pairs[1], ("SKOCI_TOKEN".to_string(), "sk_test_token_123".to_string()));
        assert_eq!(pairs[2], ("OTHER_VAL".to_string(), "single-quoted".to_string()));
        assert_eq!(pairs[3], ("EXPORTED".to_string(), "val123".to_string()));
        assert_eq!(pairs[4], ("EMPTY_LINE".to_string(), "".to_string()));
        assert_eq!(pairs[5], ("SPACED".to_string(), "spaced_val".to_string()));
    }

    #[test]
    fn test_is_local_url() {
        assert!(is_local_url("http://127.0.0.1:8787"));
        assert!(is_local_url("http://localhost:8787"));
        assert!(is_local_url("http://[::1]:8787"));
        assert!(is_local_url("https://127.0.0.1"));
        assert!(is_local_url("https://localhost"));
        assert!(is_local_url("https://[::1]"));

        // Host spoofing & bypass attempts must be rejected
        assert!(!is_local_url("https://127.0.0.1.attacker.tld"));
        assert!(!is_local_url("http://127.0.0.1.attacker.tld:8787"));
        assert!(!is_local_url("http://localhost.evil.com"));
        assert!(!is_local_url("http://attacker.com/127.0.0.1"));
        assert!(!is_local_url("http://attacker.com?url=localhost"));
        assert!(!is_local_url("http://127.0.0.2:8787"));
        assert!(!is_local_url("file:///127.0.0.1"));
        assert!(!is_local_url("not a url"));
    }

    #[test]
    fn test_filter_skoci_env_pairs_allowlist() {
        let sample = r#"
SKOCI_SERVER_URL=http://localhost:8787
SKOCI_TOKEN=sk_token_123
SKOCI_SERVER_PIN=sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
LD_PRELOAD=/evil/lib.so
DYLD_INSERT_LIBRARIES=/evil/lib.dylib
PATH=/malicious/bin
GIT_CONFIG=/evil/gitconfig
HTTP_PROXY=http://attacker.proxy
"#;
        let pairs = parse_env_file_content(sample);
        let filtered = filter_skoci_env_pairs(pairs);
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0], ("SKOCI_SERVER_URL".to_string(), "http://localhost:8787".to_string()));
        assert_eq!(filtered[1], ("SKOCI_TOKEN".to_string(), "sk_token_123".to_string()));
        assert_eq!(
            filtered[2],
            (
                "SKOCI_SERVER_PIN".to_string(),
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
            )
        );
    }

    #[test]
    fn test_local_master_token_absent_and_present() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let absent_path = temp_dir.join(format!("absent_skoci_env_{unique_id}.env"));
        assert_eq!(local_master_token_from_path(&absent_path), None);

        let present_path = temp_dir.join(format!("present_skoci_env_{unique_id}.env"));
        let sample = "OTHER_VAR=xyz\nSKOCI_MASTER_TOKEN=tok_super_secret_999\nANOTHER=abc\n";
        std::fs::write(&present_path, sample).unwrap();
        assert_eq!(
            local_master_token_from_path(&present_path),
            Some("tok_super_secret_999".to_string())
        );
        let _ = std::fs::remove_file(present_path);
    }

    #[test]
    fn test_read_project_env() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let env_path = temp_dir.join(format!("proj_env_{unique_id}.env"));
        let content = "SKOCI_SERVER_URL=https://1.2.3.4:8443\nSKOCI_TOKEN=mytoken\nSKOCI_SERVER_PIN=sha256:abcd\nDISALLOWED=true\n";
        std::fs::write(&env_path, content).unwrap();

        let parsed = read_project_env_from_path(&env_path);
        assert_eq!(
            parsed,
            Some((
                "https://1.2.3.4:8443".to_string(),
                "mytoken".to_string(),
                Some("sha256:abcd".to_string())
            ))
        );
        let _ = std::fs::remove_file(env_path);
    }

    #[test]
    fn test_read_project_env_precedence() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_env_prec_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        // 1. Only legacy .env.skoci exists
        let legacy_file = root.join(LEGACY_ENV_PATH);
        std::fs::write(&legacy_file, "SKOCI_SERVER_URL=https://legacy.example.com\nSKOCI_TOKEN=legacy_token\n").unwrap();
        let parsed = read_project_env_at(&root).unwrap();
        assert_eq!(parsed.0, "https://legacy.example.com");
        assert_eq!(parsed.1, "legacy_token");

        // 2. Both .skoci/env and legacy .env.skoci exist: .skoci/env wins
        let skoci_dir = root.join(PROJECT_DIR);
        std::fs::create_dir_all(&skoci_dir).unwrap();
        let modern_file = root.join(PROJECT_ENV_PATH);
        std::fs::write(&modern_file, "SKOCI_SERVER_URL=https://modern.example.com\nSKOCI_TOKEN=modern_token\n").unwrap();
        let parsed = read_project_env_at(&root).unwrap();
        assert_eq!(parsed.0, "https://modern.example.com");
        assert_eq!(parsed.1, "modern_token");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_write_project_env_mode_and_directory() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_write_env_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        write_project_env_at(
            &root,
            "https://server.example.com",
            "my_secret_token",
            Some("sha256:0123456789abcdef"),
        )
        .unwrap();

        let target = root.join(PROJECT_ENV_PATH);
        assert!(target.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&target).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        let parsed = read_project_env_at(&root).unwrap();
        assert_eq!(parsed.0, "https://server.example.com");
        assert_eq!(parsed.1, "my_secret_token");
        assert_eq!(parsed.2.as_deref(), Some("sha256:0123456789abcdef"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_ensure_gitignore_ignores_env() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_gitig_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        // Creates .gitignore if absent
        ensure_gitignore_ignores_env(&root).unwrap();
        let gitignore = root.join(".gitignore");
        assert!(gitignore.exists());
        let content = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(content.trim(), PROJECT_ENV_PATH);

        // Does not duplicate if already present
        ensure_gitignore_ignores_env(&root).unwrap();
        let content2 = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(content2.trim(), PROJECT_ENV_PATH);

        // Appends to existing .gitignore with other content
        std::fs::write(&gitignore, "node_modules\ntarget").unwrap();
        ensure_gitignore_ignores_env(&root).unwrap();
        let content3 = std::fs::read_to_string(&gitignore).unwrap();
        assert!(content3.contains("node_modules"));
        assert!(content3.contains(PROJECT_ENV_PATH));

        let _ = std::fs::remove_dir_all(&root);
    }
}
