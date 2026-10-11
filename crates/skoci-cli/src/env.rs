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
            let mut val = v.trim();
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val = &val[1..val.len() - 1];
            }
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
                || h.eq_ignore_ascii_case("::1")
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
        if k == "SKOCI_SERVER_URL" || k == "SKOCI_TOKEN" {
            filtered.push((k, v));
        } else {
            eprintln!("Warning: Ignoring disallowed key '{k}' in .env.skoci (only SKOCI_SERVER_URL and SKOCI_TOKEN are allowed)");
        }
    }
    filtered
}

pub fn load_env_files() {
    let mut server_url_set_by_env_skoci = false;
    let cwd_env = Path::new(".env.skoci");
    if cwd_env.exists() {
        if let Ok(content) = std::fs::read_to_string(cwd_env) {
            let pairs = filter_skoci_env_pairs(parse_env_file_content(&content));
            for (k, v) in pairs {
                if std::env::var(&k).is_err() {
                    std::env::set_var(&k, &v);
                    if k == "SKOCI_SERVER_URL" {
                        server_url_set_by_env_skoci = true;
                    }
                }
            }
        }
    }

    if std::env::var("SKOCI_TOKEN").is_err() && !server_url_set_by_env_skoci {
        let server_url = std::env::var("SKOCI_SERVER_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
        let is_local = is_local_url(&server_url);

        if is_local {
            let etc_path = Path::new("/etc/skoci/skoci.env");
            if etc_path.exists() {
                match std::fs::read_to_string(etc_path) {
                    Ok(content) => {
                        for (k, v) in parse_env_file_content(&content) {
                            if k == "SKOCI_MASTER_TOKEN" && std::env::var("SKOCI_TOKEN").is_err() {
                                std::env::set_var("SKOCI_TOKEN", &v);
                            }
                            if std::env::var(&k).is_err() {
                                std::env::set_var(&k, &v);
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        eprintln!("Permission denied reading /etc/skoci/skoci.env: {e}");
                        eprintln!(
                            "Fix: sudo usermod -aG skoci $USER then log in again, or use sudo."
                        );
                    }
                    Err(_) => {}
                }
            }
        }
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
LD_PRELOAD=/evil/lib.so
DYLD_INSERT_LIBRARIES=/evil/lib.dylib
PATH=/malicious/bin
GIT_CONFIG=/evil/gitconfig
HTTP_PROXY=http://attacker.proxy
"#;
        let pairs = parse_env_file_content(sample);
        let filtered = filter_skoci_env_pairs(pairs);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0], ("SKOCI_SERVER_URL".to_string(), "http://localhost:8787".to_string()));
        assert_eq!(filtered[1], ("SKOCI_TOKEN".to_string(), "sk_token_123".to_string()));
    }
}
