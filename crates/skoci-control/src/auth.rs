use chrono::Utc;
use skoci_api::error::{SkociError, SkociResult};
use crate::db::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Admin,
    Agent {
        app: String,
        name: String,
        id: String,
    },
}

impl Subject {
    pub fn identifier(&self) -> String {
        match self {
            Subject::Admin => "admin".to_string(),
            Subject::Agent { app, name, id } => format!("agent:{app}/{name}#{id}"),
        }
    }

    pub fn actor_type(&self) -> &'static str {
        match self {
            Subject::Admin => "admin",
            Subject::Agent { .. } => "agent",
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, Subject::Admin)
    }

    pub fn can_access_app(&self, requested_app: &str) -> bool {
        match self {
            Subject::Admin => true,
            Subject::Agent { app, .. } => app == requested_app,
        }
    }

    pub fn require_app(&self, requested_app: &str) -> SkociResult<()> {
        if self.can_access_app(requested_app) {
            Ok(())
        } else {
            match self {
                Subject::Agent { app, .. } => Err(SkociError::scope_violation(app, requested_app)),
                Subject::Admin => Ok(()),
            }
        }
    }

    pub fn require_admin(&self) -> SkociResult<()> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(SkociError::forbidden(
                "This action requires admin privileges",
                "Use an admin / master token to perform this action",
            ))
        }
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_agent_token(app: &str) -> (String, String, String) {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token_hex = hex::encode(bytes);
    let token = format!("sk_agent_{app}_{token_hex}");
    let hash = hash_token(&token);
    let id = hash[..12].to_string();
    (token, hash, id)
}

pub fn authenticate_token(db: &Database, master_token: &str, token: &str) -> SkociResult<Subject> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(SkociError::unauthorized("Authorization token is empty"));
    }

    if constant_time_eq(trimmed.as_bytes(), master_token.as_bytes()) {
        return Ok(Subject::Admin);
    }

    if trimmed.starts_with("sk_agent_") {
        let hash = hash_token(trimmed);
        if let Some(record) = db.get_agent_token_by_hash(&hash)? {
            let id = if hash.len() >= 12 {
                hash[..12].to_string()
            } else {
                hash.clone()
            };
            let now = Utc::now();
            let should_update = match record.last_used_at {
                Some(last) => (now - last).num_seconds() >= 60,
                None => true,
            };
            if should_update {
                let _ = db.update_token_last_used_at(&hash, now);
            }
            return Ok(Subject::Agent {
                app: record.app,
                name: record.name,
                id,
            });
        }
    }

    Err(SkociError::unauthorized("Token is invalid or has been revoked"))
}
