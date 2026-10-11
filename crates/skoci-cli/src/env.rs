use std::path::Path;

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

pub fn filter_skoci_env_pairs(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut filtered = Vec::new();
    for (k, v) in pairs {
        if k == "SKOCI_SERVER_URL" || k == "SKOCI_TOKEN" || k == "SKOCI_SERVER_PIN" {
            filtered.push((k, v));
        } else {
            eprintln!("Warning: Ignoring disallowed key '{k}' in .env.skoci (only SKOCI_SERVER_URL, SKOCI_TOKEN, and SKOCI_SERVER_PIN are allowed)");
        }
    }
    filtered
}

pub fn load_env_files() {
    let cwd_env = Path::new(".env.skoci");
    if cwd_env.exists() {
        if let Ok(content) = std::fs::read_to_string(cwd_env) {
            let pairs = filter_skoci_env_pairs(parse_env_file_content(&content));
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
    read_project_env_from_path(Path::new(".env.skoci"))
}

pub fn read_project_env_from_path(path: &Path) -> Option<(String, String, Option<String>)> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let pairs = filter_skoci_env_pairs(parse_env_file_content(&content));
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
}
