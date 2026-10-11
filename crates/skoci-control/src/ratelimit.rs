use parking_lot::Mutex;
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct AuthRateLimiter {
    failures: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthRateLimiter {
    pub fn new() -> Self {
        Self {
            failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, ip: IpAddr) -> SkociResult<()> {
        let mut map = self.failures.lock();
        let now = Instant::now();
        let window = Duration::from_secs(60);

        if let Some(times) = map.get_mut(&ip) {
            times.retain(|&t| now.duration_since(t) <= window);
            if times.len() >= 10 {
                return Err(SkociError::rate_limited());
            }
        }
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let mut map = self.failures.lock();
        let now = Instant::now();
        let window = Duration::from_secs(60);

        let times = map.entry(ip).or_default();
        times.retain(|&t| now.duration_since(t) <= window);
        times.push(now);
    }
}
