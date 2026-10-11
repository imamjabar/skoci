use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

pub const MAX_SESSIONS: usize = 1_000;

#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn len(&self) -> usize {
        self.sessions.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.read().is_empty()
    }
    pub fn create_session(&self) -> (String, DateTime<Utc>) {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let id = hex::encode(bytes);
        let expires_at = Utc::now() + Duration::hours(12);

        let mut lock = self.sessions.write();
        self.cleanup_expired_locked(&mut lock);
        if lock.len() >= MAX_SESSIONS {
            let oldest_key = lock
                .iter()
                .min_by_key(|(_, expires)| **expires)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest_key {
                lock.remove(&k);
            }
        }
        lock.insert(id.clone(), expires_at);
        (id, expires_at)
    }

    pub fn validate_session(&self, id: &str) -> bool {
        let lock = self.sessions.read();
        if let Some(&expires_at) = lock.get(id) {
            expires_at > Utc::now()
        } else {
            false
        }
    }

    pub fn delete_session(&self, id: &str) {
        let mut lock = self.sessions.write();
        lock.remove(id);
    }

    fn cleanup_expired_locked(&self, map: &mut HashMap<String, DateTime<Utc>>) {
        let now = Utc::now();
        map.retain(|_, expires_at| *expires_at > now);
    }
}
