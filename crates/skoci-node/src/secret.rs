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
    pub fn load_or_create(key_path: &Path) -> SkociResult<Self> {
        if key_path.exists() {
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
                    "Delete the corrupted key file so it can be regenerated automatically",
                ));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(Self::from_key(key))
        } else {
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
            opts.write(true).create_new(true);

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

            Ok(Self::from_key(key))
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

pub fn redact(text: &str, secret_values: &[String]) -> String {
    let mut sorted_secrets: Vec<&str> = secret_values
        .iter()
        .map(|s| s.as_str())
        .filter(|s| s.len() >= 4)
        .collect();

    sorted_secrets.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    sorted_secrets.dedup();

    let mut result = text.to_string();
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
}
