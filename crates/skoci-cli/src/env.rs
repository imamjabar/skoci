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

pub fn load_env_files() {
    let cwd_env = Path::new(".env.skoci");
    if cwd_env.exists() {
        if let Ok(content) = std::fs::read_to_string(cwd_env) {
            for (k, v) in parse_env_file_content(&content) {
                if std::env::var(&k).is_err() {
                    std::env::set_var(&k, &v);
                }
            }
        }
    }

    if std::env::var("SKOCI_TOKEN").is_err() {
        let server_url = std::env::var("SKOCI_SERVER_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
        let is_local = server_url.contains("127.0.0.1") || server_url.contains("localhost");

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
                        eprintln!("Izin ditolak membaca /etc/skoci/skoci.env: {e}");
                        eprintln!(
                            "Solusi: sudo usermod -aG skoci $USER lalu login ulang, atau gunakan sudo."
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
# Ini komentar
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
}
