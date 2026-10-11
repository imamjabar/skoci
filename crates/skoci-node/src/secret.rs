use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSecret {
    pub key: String,
    pub ciphertext_hex: String,
    pub nonce_hex: String,
}

pub struct SecretStore {
    cipher_key: Key,
    secrets: HashMap<String, EncryptedSecret>, // app -> secrets
}

impl SecretStore {
    pub fn new(master_seed: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(master_seed.as_bytes());
        let key_bytes = hasher.finalize();
        let cipher_key = *Key::from_slice(&key_bytes);

        Self {
            cipher_key,
            secrets: HashMap::new(),
        }
    }

    pub fn set_secret(&mut self, app: &str, key: &str, raw_value: &str) -> SkociResult<()> {
        let cipher = ChaCha20Poly1305::new(&self.cipher_key);

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, raw_value.as_bytes())
            .map_err(|e| SkociError::bad_request(format!("Enkripsi secret gagal: {e}"), "Coba lagi"))?;

        let entry = EncryptedSecret {
            key: key.to_string(),
            ciphertext_hex: hex::encode(ciphertext),
            nonce_hex: hex::encode(nonce_bytes),
        };

        let storage_key = format!("{app}:{key}");
        self.secrets.insert(storage_key, entry);
        Ok(())
    }

    pub fn get_encrypted(&self, app: &str, key: &str) -> Option<&EncryptedSecret> {
        let storage_key = format!("{app}:{key}");
        self.secrets.get(&storage_key)
    }

    pub fn load_encrypted(&mut self, app: &str, key: &str, ciphertext_hex: String, nonce_hex: String) {
        let storage_key = format!("{app}:{key}");
        self.secrets.insert(
            storage_key,
            EncryptedSecret {
                key: key.to_string(),
                ciphertext_hex,
                nonce_hex,
            },
        );
    }

    pub fn get_secret_decrypted(&self, app: &str, key: &str) -> SkociResult<String> {
        let storage_key = format!("{app}:{key}");
        let entry = self
            .secrets
            .get(&storage_key)
            .ok_or_else(|| SkociError::secret_missing(key))?;

        let cipher = ChaCha20Poly1305::new(&self.cipher_key);
        let nonce_bytes = hex::decode(&entry.nonce_hex)
            .map_err(|e| SkociError::bad_request(format!("Invalid nonce: {e}"), "Secret rusak"))?;
        let ciphertext = hex::decode(&entry.ciphertext_hex)
            .map_err(|e| SkociError::bad_request(format!("Invalid ciphertext: {e}"), "Secret rusak"))?;

        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| SkociError::bad_request(format!("Dekripsi secret gagal: {e}"), "Kunci tidak cocok"))?;

        String::from_utf8(plaintext)
            .map_err(|e| SkociError::bad_request(format!("UTF-8 decode error: {e}"), "Secret bukan string valid"))
    }

    pub fn list_keys(&self, app: &str) -> Vec<String> {
        let prefix = format!("{app}:");
        self.secrets
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect()
    }

    pub fn redact_logs(&self, app: &str, log_text: &str) -> String {
        let mut sanitized = log_text.to_string();
        for key in self.list_keys(app) {
            if let Ok(raw) = self.get_secret_decrypted(app, &key) {
                if !raw.trim().is_empty() {
                    sanitized = sanitized.replace(&raw, "[REDACTED_SECRET]");
                }
            }
        }
        sanitized
    }
}
