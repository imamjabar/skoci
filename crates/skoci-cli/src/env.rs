use std::path::Path;
use skoci_api::models::{PROJECT_DIR, PROJECT_ENV_PATH};
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

const PROJECT_ENV_CREDENTIAL_KEYS: [&str; 3] =
    ["SKOCI_SERVER_URL", "SKOCI_TOKEN", "SKOCI_SERVER_PIN"];

pub fn load_env_files() {
    load_env_files_at(Path::new("."));
}

pub fn load_env_files_at(root: &Path) {
    let modern = root.join(PROJECT_ENV_PATH);
    if modern.exists() {
        if let Ok(content) = std::fs::read_to_string(&modern) {
            let pairs = filter_skoci_env_pairs_with_source(
                parse_env_file_content(&content),
                &modern.display().to_string(),
            );
            for (k, v) in pairs {
                // Credentials stay out of the process environment: the resolver takes them as an
                // explicit project-env layer, which is what makes a flag outrank the file and what
                // lets the CLI report which store a command actually used.
                if PROJECT_ENV_CREDENTIAL_KEYS.contains(&k.as_str()) {
                    continue;
                }
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
            eprintln!("Fix: Re-run the command with sudo (e.g. 'sudo skoci approve <id>').");
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o700));
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

pub fn migrate_legacy_project_layout(root: &Path) -> SkociResult<Option<String>> {
    use skoci_api::models::{LEGACY_CONFIG_PATH, LEGACY_ENV_PATH, PROJECT_CONFIG_PATH};

    let legacy_cfg = root.join(LEGACY_CONFIG_PATH);
    let legacy_env = root.join(LEGACY_ENV_PATH);
    let has_legacy_cfg = legacy_cfg.is_file();
    let has_legacy_env = legacy_env.is_file();

    if !has_legacy_cfg && !has_legacy_env {
        return Ok(None);
    }

    let modern_cfg = root.join(PROJECT_CONFIG_PATH);
    let modern_env = root.join(PROJECT_ENV_PATH);
    let need_move_cfg = has_legacy_cfg && !modern_cfg.exists();
    let need_move_env = has_legacy_env && !modern_env.exists();

    if need_move_cfg || need_move_env {
        let project_dir = root.join(PROJECT_DIR);
        if !project_dir.exists() {
            std::fs::create_dir_all(&project_dir).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create {} directory: {e}", project_dir.display()),
                    "Check directory write permissions",
                )
            })?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o700));
        }
    }

    let mut actions = Vec::new();

    if has_legacy_cfg {
        if modern_cfg.exists() {
            actions.push(format!(
                "kept existing {PROJECT_CONFIG_PATH} (left legacy {LEGACY_CONFIG_PATH} untouched)"
            ));
        } else {
            std::fs::rename(&legacy_cfg, &modern_cfg)
                .or_else(|_| {
                    std::fs::copy(&legacy_cfg, &modern_cfg)?;
                    std::fs::remove_file(&legacy_cfg)
                })
                .map_err(|e| {
                    SkociError::internal(
                        format!(
                            "Failed to migrate {LEGACY_CONFIG_PATH} to {PROJECT_CONFIG_PATH}: {e}"
                        ),
                        "Check directory write permissions",
                    )
                })?;
            actions.push(format!("{LEGACY_CONFIG_PATH} -> {PROJECT_CONFIG_PATH}"));
        }
    }

    if has_legacy_env {
        if modern_env.exists() {
            actions.push(format!(
                "kept existing {PROJECT_ENV_PATH} (left legacy {LEGACY_ENV_PATH} untouched)"
            ));
        } else {
            let content = std::fs::read(&legacy_env).map_err(|e| {
                SkociError::internal(
                    format!("Failed to read legacy {LEGACY_ENV_PATH}: {e}"),
                    "Check file read permissions",
                )
            })?;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            let mut f = opts.open(&modern_env).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create {PROJECT_ENV_PATH}: {e}"),
                    "Check write permissions in .skoci directory",
                )
            })?;
            f.write_all(&content).map_err(|e| {
                let _ = std::fs::remove_file(&modern_env);
                SkociError::internal(
                    format!("Failed to write {PROJECT_ENV_PATH}: {e}"),
                    "Check disk space and permissions",
                )
            })?;
            drop(f);
            std::fs::remove_file(&legacy_env).map_err(|e| {
                SkociError::internal(
                    format!("Failed to remove legacy {LEGACY_ENV_PATH}: {e}"),
                    "Check directory write permissions",
                )
            })?;
            let _ = ensure_gitignore_ignores_env(root);
            actions.push(format!("{LEGACY_ENV_PATH} -> {PROJECT_ENV_PATH}"));
        }
    }

    if actions.is_empty() {
        Ok(None)
    } else {
        let line = format!("Migrated legacy project layout: {}", actions.join(", "));
        eprintln!("{line}");
        Ok(Some(line))
    }
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
    fn test_read_project_env_reads_modern_only() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_env_modern_only_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        // Legacy .env.skoci is not read directly by read_project_env_at
        let legacy_file = root.join(".env.skoci");
        std::fs::write(
            &legacy_file,
            "SKOCI_SERVER_URL=https://legacy.example.com\nSKOCI_TOKEN=legacy_token\n",
        )
        .unwrap();
        assert_eq!(read_project_env_at(&root), None);

        // Modern .skoci/env is read
        let skoci_dir = root.join(PROJECT_DIR);
        std::fs::create_dir_all(&skoci_dir).unwrap();
        let modern_file = root.join(PROJECT_ENV_PATH);
        std::fs::write(
            &modern_file,
            "SKOCI_SERVER_URL=https://modern.example.com\nSKOCI_TOKEN=modern_token\n",
        )
        .unwrap();
        let parsed = read_project_env_at(&root).unwrap();
        assert_eq!(parsed.0, "https://modern.example.com");
        assert_eq!(parsed.1, "modern_token");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_migrate_legacy_project_layout() {
        let temp_dir = std::env::temp_dir();
        let unique_id = uuid::Uuid::new_v4().simple();
        let root = temp_dir.join(format!("test_migrate_layout_{unique_id}"));
        std::fs::create_dir_all(&root).unwrap();

        // 1. Modern-only or empty project is untouched
        assert_eq!(migrate_legacy_project_layout(&root).unwrap(), None);
        assert!(!root.join(".skoci").exists());

        // 2. Legacy skoci.yaml and .env.skoci move to .skoci/skoci.yaml and .skoci/env
        std::fs::write(root.join("skoci.yaml"), "version: 1\napp: migrated-app\n").unwrap();
        std::fs::write(
            root.join(".env.skoci"),
            "SKOCI_SERVER_URL=https://1.2.3.4:8443\nSKOCI_TOKEN=tok_migrated\n",
        )
        .unwrap();

        let msg = migrate_legacy_project_layout(&root).unwrap().unwrap();
        assert!(msg.contains("skoci.yaml -> .skoci/skoci.yaml"));
        assert!(msg.contains(".env.skoci -> .skoci/env"));
        assert!(!root.join("skoci.yaml").exists());
        assert!(!root.join(".env.skoci").exists());
        assert_eq!(
            std::fs::read_to_string(root.join(".skoci/skoci.yaml")).unwrap(),
            "version: 1\napp: migrated-app\n"
        );
        let parsed = read_project_env_at(&root).unwrap();
        assert_eq!(parsed.0, "https://1.2.3.4:8443");
        assert_eq!(parsed.1, "tok_migrated");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(root.join(".skoci")).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700);
            let env_mode = std::fs::metadata(root.join(".skoci/env")).unwrap().permissions().mode() & 0o777;
            assert_eq!(env_mode, 0o600);
        }

        // 3. Re-running on already-modern project is a no-op
        assert_eq!(migrate_legacy_project_layout(&root).unwrap(), None);

        // 4. When both modern and legacy exist, modern is kept and legacy is left untouched
        std::fs::write(root.join("skoci.yaml"), "version: 1\napp: stale-legacy\n").unwrap();
        std::fs::write(
            root.join(".env.skoci"),
            "SKOCI_SERVER_URL=https://stale\nSKOCI_TOKEN=stale\n",
        )
        .unwrap();
        let msg_both = migrate_legacy_project_layout(&root).unwrap().unwrap();
        assert!(msg_both.contains("kept existing .skoci/skoci.yaml"));
        assert!(msg_both.contains("kept existing .skoci/env"));
        assert_eq!(
            std::fs::read_to_string(root.join(".skoci/skoci.yaml")).unwrap(),
            "version: 1\napp: migrated-app\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("skoci.yaml")).unwrap(),
            "version: 1\napp: stale-legacy\n"
        );
        assert!(root.join(".env.skoci").exists());

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
