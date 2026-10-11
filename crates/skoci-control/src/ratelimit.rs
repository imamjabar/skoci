use parking_lot::Mutex;
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MAX_RATE_LIMITER_ENTRIES: usize = 50_000;

pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            let mut prefix = [0u8; 16];
            prefix[..8].copy_from_slice(&octets[..8]);
            IpAddr::V6(std::net::Ipv6Addr::from(prefix))
        }
    }
}

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

    pub fn len(&self) -> usize {
        self.failures.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.failures.lock().is_empty()
    }

    pub fn check(&self, ip: IpAddr) -> SkociResult<()> {
        let ip = normalize_ip(ip);
        let mut map = self.failures.lock();
        let now = Instant::now();
        let window = Duration::from_secs(60);

        if let Some(times) = map.get_mut(&ip) {
            times.retain(|&t| now.duration_since(t) <= window);
            if times.is_empty() {
                map.remove(&ip);
            } else if times.len() >= 10 {
                return Err(SkociError::rate_limited());
            }
        }
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let ip = normalize_ip(ip);
        let mut map = self.failures.lock();
        let now = Instant::now();
        let window = Duration::from_secs(60);

        // Sweep empty entries if map is large
        if map.len() >= MAX_RATE_LIMITER_ENTRIES && !map.contains_key(&ip) {
            map.retain(|_, times| {
                times.retain(|&t| now.duration_since(t) <= window);
                !times.is_empty()
            });
            // If still at capacity, evict oldest entry
            if map.len() >= MAX_RATE_LIMITER_ENTRIES {
                let oldest_ip = map
                    .iter()
                    .min_by_key(|(_, times)| times.first().copied().unwrap_or(now))
                    .map(|(k, _)| *k);
                if let Some(oldest) = oldest_ip {
                    map.remove(&oldest);
                }
            }
        }

        let times = map.entry(ip).or_default();
        times.retain(|&t| now.duration_since(t) <= window);
        times.push(now);
    }
}
