use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use skoci_api::error::{SkociError, SkociResult};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub struct SecretCipher {
    key: [u8; 32],
}

impl SecretCipher {
    pub fn load(key_path: &Path) -> SkociResult<Self> {
        if !key_path.exists() {
            return Err(SkociError::not_found(
                format!("Encryption key file not found at {}", key_path.display()),
                format!("Restore the backed-up node.key to {} (mode 0600)", key_path.display()),
            ));
        }
        let bytes = fs::read(key_path).map_err(|e| {
            SkociError::internal(
                format!("Failed to read encryption key file: {e}"),
                "Check read permissions on the key file",
            )
        })?;
        if bytes.len() != 32 {
            return Err(SkociError::bad_request(
                format!(
                    "Invalid encryption key file length: {} bytes (must be 32 bytes)",
                    bytes.len()
                ),
                format!(
                    "Restore the 32-byte node.key from backup to {} (mode 0600) or restart with SKOCI_ALLOW_SECRET_KEY_RESET=true to reset",
                    key_path.display()
                ),
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self::from_key(key))
    }

    pub fn create_and_persist(key_path: &Path) -> SkociResult<Self> {
        if let Some(parent) = key_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create encryption key directory: {e}"),
                    "Check directory permissions",
                )
            })?;
        }

        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let mut file = opts.open(key_path).map_err(|e| {
            SkociError::internal(
                format!("Failed to create 0600 encryption key file: {e}"),
                "Check directory permissions",
            )
        })?;

        file.write_all(&key).map_err(|e| {
            SkociError::internal(
                format!("Failed to write encryption key bytes: {e}"),
                "Check disk space or permissions",
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(key_path, fs::Permissions::from_mode(0o600));
        }

        Ok(Self::from_key(key))
    }

    pub fn load_or_create(key_path: &Path) -> SkociResult<Self> {
        if key_path.exists() {
            Self::load(key_path)
        } else {
            Self::create_and_persist(key_path)
        }
    }

    pub fn from_key(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn encrypt(&self, plaintext: &str) -> SkociResult<(String, String)> {
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|e| SkociError::internal(format!("Secret encryption failed: {e}"), "Try again"))?;

        Ok((hex::encode(ciphertext), hex::encode(nonce_bytes)))
    }

    pub fn decrypt(&self, ciphertext_hex: &str, nonce_hex: &str) -> SkociResult<String> {
        let ciphertext = hex::decode(ciphertext_hex).map_err(|e| {
            SkociError::bad_request(
                format!("Invalid ciphertext hex format: {e}"),
                "Check secret data integrity",
            )
        })?;

        let nonce_bytes = hex::decode(nonce_hex).map_err(|e| {
            SkociError::bad_request(
                format!("Invalid nonce hex format: {e}"),
                "Check nonce data integrity",
            )
        })?;

        if nonce_bytes.len() != 12 {
            return Err(SkociError::bad_request(
                format!(
                    "Invalid nonce length (must be 12 bytes, got {})",
                    nonce_bytes.len()
                ),
                "Check nonce data integrity",
            ));
        }

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let decrypted_bytes = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|e| {
                SkociError::bad_request(
                    format!("Decryption failed (wrong key or corrupted ciphertext): {e}"),
                    "Verify that the encryption key matches the data",
                )
            })?;

        String::from_utf8(decrypted_bytes).map_err(|e| {
            SkociError::bad_request(
                format!("Decrypted result is not valid UTF-8: {e}"),
                "Ensure encrypted data is valid text",
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Load,
    CreateNew,
    RecreateAndWipe,
}

pub fn decide_key_action(
    key_path: &Path,
    key_valid: bool,
    has_secrets: bool,
    allow_reset: bool,
) -> SkociResult<KeyAction> {
    if key_valid {
        Ok(KeyAction::Load)
    } else if has_secrets {
        if allow_reset {
            Ok(KeyAction::RecreateAndWipe)
        } else {
            Err(SkociError::internal(
                format!(
                    "Encryption key file {} is missing or unreadable, but stored secrets exist in the database. Secrets cannot be decrypted without the original key.",
                    key_path.display()
                ),
                format!(
                    "Restore the backed-up key to {} (mode 0600), or restart with SKOCI_ALLOW_SECRET_KEY_RESET=true to recreate it and delete the unrecoverable secrets.",
                    key_path.display()
                ),
            ))
        }
    } else {
        Ok(KeyAction::CreateNew)
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle_bytes = needle.as_bytes();
    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return None;
    }
    for i in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        if haystack.is_char_boundary(i)
            && haystack_bytes[i..i + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes)
        {
            return Some(i);
        }
    }
    None
}

fn redact_skoci_tokens(mut text: String) -> String {
    for prefix in &["sk_agent_", "sk_master_"] {
        let mut search_from = 0;
        while let Some(rel_idx) = text[search_from..].find(prefix) {
            let start = search_from + rel_idx;
            let after_prefix = start + prefix.len();
            let token_end_offset = text[after_prefix..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(text[after_prefix..].len());
            let end = after_prefix + token_end_offset;
            text.replace_range(start..end, "[REDACTED]");
            search_from = start + "[REDACTED]".len();
            if search_from >= text.len() {
                break;
            }
        }
    }
    text
}

fn redact_bearer_headers(mut text: String) -> String {
    let mut search_from = 0;
    loop {
        if search_from >= text.len() {
            break;
        }
        let Some(rel_idx) = find_ascii_case_insensitive(&text[search_from..], "bearer") else {
            break;
        };

        let bearer_start = search_from + rel_idx;
        let after_bearer = bearer_start + "bearer".len();

        if after_bearer < text.len() {
            let next_char = text[after_bearer..].chars().next().unwrap();
            if !(next_char.is_whitespace() || next_char == ':') {
                search_from = after_bearer;
                continue;
            }
        } else {
            break;
        }

        let rest = &text[after_bearer..];
        let token_offset = rest
            .find(|c: char| !c.is_whitespace() && c != ':' && c != '"' && c != '\'')
            .unwrap_or(rest.len());
        let val_start = after_bearer + token_offset;

        if val_start >= text.len() {
            break;
        }

        if text[val_start..].starts_with("[REDACTED]") {
            search_from = val_start + "[REDACTED]".len();
            continue;
        }

        let prefix = &text[..bearer_start];
        let is_auth_header = find_ascii_case_insensitive(prefix, "authorization").is_some();
        let is_capital_bearer = text[bearer_start..].starts_with("Bearer");

        if is_auth_header || is_capital_bearer {
            let val_rest = &text[val_start..];
            let val_len = val_rest
                .find(|c: char| {
                    c.is_whitespace()
                        || c == '"'
                        || c == '\''
                        || c == ','
                        || c == ';'
                        || c == '}'
                        || c == ')'
                })
                .unwrap_or(val_rest.len());

            if val_len > 0 {
                let token_candidate = &val_rest[..val_len];
                let is_common_word = !is_auth_header
                    && matches!(
                        token_candidate.to_ascii_lowercase().as_str(),
                        "of" | "the" | "and" | "a" | "in" | "to" | "bonds" | "bond" | "token" | "format"
                    );
                if !is_common_word {
                    text.replace_range(val_start..val_start + val_len, "[REDACTED]");
                    search_from = val_start + "[REDACTED]".len();
                    continue;
                }
            }
        }

        search_from = val_start;
    }
    text
}

pub fn redact(text: &str, secret_values: &[String]) -> String {
    let mut result = text.to_string();

    result = redact_skoci_tokens(result);
    result = redact_bearer_headers(result);

    let mut sorted_secrets: Vec<&str> = secret_values
        .iter()
        .map(|s| s.as_str())
        .filter(|s| s.len() >= 4)
        .collect();

    sorted_secrets.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    sorted_secrets.dedup();

    for secret in sorted_secrets {
        result = result.replace(secret, "[REDACTED]");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let key = [42u8; 32];
        let cipher = SecretCipher::from_key(key);
        let secret = "rahasia_negara_123456789";

        let (ct, nonce) = cipher.encrypt(secret).unwrap();
        let decrypted = cipher.decrypt(&ct, &nonce).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let cipher1 = SecretCipher::from_key(key1);
        let cipher2 = SecretCipher::from_key(key2);

        let (ct, nonce) = cipher1.encrypt("pesan_rahasia").unwrap();
        let res = cipher2.decrypt(&ct, &nonce);
        assert!(res.is_err());
    }

    #[test]
    fn test_key_file_persisted_and_reloaded() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_key_test_{:x}", rand::random::<u64>()));
        let key_path = temp_dir.join("sub").join("node.key");

        let cipher1 = SecretCipher::load_or_create(&key_path).unwrap();
        let secret = "kunci_tersimpan_dengan_baik";
        let (ct, nonce) = cipher1.encrypt(secret).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let cipher2 = SecretCipher::load_or_create(&key_path).unwrap();
        let decrypted = cipher2.decrypt(&ct, &nonce).unwrap();
        assert_eq!(decrypted, secret);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_redact() {
        let secrets = vec![
            "123".to_string(), // < 4 chars, ignored
            "password".to_string(),
            "password123".to_string(), // longer, should be replaced first
        ];

        let log = "Connecting with password password123 and port 123";
        let redacted = redact(log, &secrets);

        assert_eq!(redacted, "Connecting with [REDACTED] [REDACTED] and port 123");
    }

    #[test]
    fn test_redact_tokens_and_bearer() {
        let log1 = "Agent token is sk_agent_x and master is sk_master_x.";
        assert_eq!(
            redact(log1, &[]),
            "Agent token is [REDACTED] and master is [REDACTED]."
        );

        let log2 = "Authorization: Bearer secret_api_token_12345 in request";
        assert_eq!(
            redact(log2, &[]),
            "Authorization: Bearer [REDACTED] in request"
        );

        let log3 = "authorization: bearer my_bearer_token";
        assert_eq!(
            redact(log3, &[]),
            "authorization: bearer [REDACTED]"
        );

        let log4 = "The bearer of the certificate must present identification to continue";
        assert_eq!(redact(log4, &[]), log4);

        let log5 = "Application started on port 8080 with 4 workers successfully";
        assert_eq!(redact(log5, &[]), log5);
    }

    #[test]
    fn test_key_loss_decision() {
        let key_path = Path::new("/var/lib/skoci/node.key");
        let res1 = decide_key_action(key_path, false, true, false);
        assert!(res1.is_err());
        let err = res1.unwrap_err();
        assert!(err.message.contains("missing or unreadable"));
        assert!(err.fix.contains("/var/lib/skoci/node.key"));
        assert!(err.fix.contains("SKOCI_ALLOW_SECRET_KEY_RESET=true"));

        let res2 = decide_key_action(key_path, false, false, false);
        assert_eq!(res2.unwrap(), KeyAction::CreateNew);

        let res3 = decide_key_action(key_path, false, true, true);
        assert_eq!(res3.unwrap(), KeyAction::RecreateAndWipe);

        let res4 = decide_key_action(key_path, true, true, false);
        assert_eq!(res4.unwrap(), KeyAction::Load);

        let res5 = decide_key_action(key_path, true, false, false);
        assert_eq!(res5.unwrap(), KeyAction::Load);
    }
}
