use rand::RngCore;
use sha2::{Digest, Sha256};
use skoci_api::error::{SkociError, SkociResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSubject {
    Human { user: String },
    Agent { token_id: String, app: String },
}

impl AuthSubject {
    pub fn is_human(&self) -> bool {
        matches!(self, AuthSubject::Human { .. })
    }

    pub fn identifier(&self) -> &str {
        match self {
            AuthSubject::Human { user } => user,
            AuthSubject::Agent { token_id, .. } => token_id,
        }
    }

    pub fn can_access_app(&self, app_name: &str) -> bool {
        match self {
            AuthSubject::Human { .. } => true,
            AuthSubject::Agent { app, .. } => app == "all" || app == app_name,
        }
    }
}

pub struct AuthManager {
    master_admin_token: String,
}

impl AuthManager {
    pub fn new(master_admin_token: impl Into<String>) -> Self {
        Self {
            master_admin_token: master_admin_token.into(),
        }
    }

    pub fn generate_token(prefix: &str) -> String {
        let mut bytes = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut bytes);
        format!("{prefix}_{}", hex::encode(bytes))
    }

    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn verify_token(&self, header_val: &str) -> SkociResult<AuthSubject> {
        let token = header_val.trim().strip_prefix("Bearer ").unwrap_or(header_val.trim());

        if token == self.master_admin_token {
            return Ok(AuthSubject::Human {
                user: "admin".to_string(),
            });
        }

        if let Some(rest) = token.strip_prefix("sk_agent_") {
            // If token has shape sk_agent_<app>_<hex>, extract app
            let parts: Vec<&str> = rest.split('_').collect();
            let app = if parts.len() >= 2 {
                parts[0].to_string()
            } else {
                "all".to_string()
            };
            Ok(AuthSubject::Agent {
                token_id: token.to_string(),
                app,
            })
        } else if token.starts_with("sk_human_") {
            Ok(AuthSubject::Human {
                user: "operator".to_string(),
            })
        } else {
            Err(SkociError::unauthorized("Token otentikasi tidak valid atau tidak dikenal"))
        }
    }
}
