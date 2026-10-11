use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub app: String,
    pub env: String,
    pub hosts: Vec<String>,
    pub target: SocketAddr,
    pub version: String,
}

impl Route {
    pub fn key(&self) -> String {
        format!("{}:{}", self.app, self.env)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    App { key: String, target: SocketAddr },
    Control(SocketAddr),
    NotFound,
}

#[derive(Clone)]
pub struct ProxyRouter {
    inner: Arc<RwLock<RouterInner>>,
    gen_tx: watch::Sender<u64>,
}

struct RouterInner {
    routes: HashMap<String, Route>,
    host_map: HashMap<String, String>,
    default_app: Option<String>,
    control_host: Option<String>,
    control_addr: Option<SocketAddr>,
}

impl Default for ProxyRouter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn clean_hostname(host: &str) -> String {
    let trimmed = host.trim();
    if trimmed.starts_with('[') {
        if let Some(idx) = trimmed.find(']') {
            return trimmed[..=idx].to_ascii_lowercase();
        }
    }
    trimmed.split(':').next().unwrap_or("").to_ascii_lowercase()
}

pub fn is_ip_literal_or_empty(host: &str) -> bool {
    let clean = clean_hostname(host);
    if clean.is_empty() {
        return true;
    }
    if clean.parse::<IpAddr>().is_ok() {
        return true;
    }
    if clean.starts_with('[') && clean.ends_with(']') && clean.len() >= 2
        && clean[1..clean.len() - 1].parse::<IpAddr>().is_ok() {
            return true;
        }
    false
}

pub fn is_dns_name(host: &str) -> bool {
    let clean = clean_hostname(host);
    if clean.is_empty() || clean == "localhost" {
        return false;
    }
    !is_ip_literal_or_empty(&clean)
}

impl ProxyRouter {
    pub fn new() -> Self {
        let (gen_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(RwLock::new(RouterInner {
                routes: HashMap::new(),
                host_map: HashMap::new(),
                default_app: None,
                control_host: None,
                control_addr: None,
            })),
            gen_tx,
        }
    }

    pub async fn upsert_route(&self, route: Route) -> Option<Route> {
        let key = route.key();
        let mut inner = self.inner.write().await;
        let prev = inner.routes.insert(key, route);

        inner.rebuild_host_map();
        self.gen_tx.send_modify(|g| *g += 1);
        prev
    }

    pub async fn remove_route(&self, key: &str) -> Option<Route> {
        let mut inner = self.inner.write().await;
        let prev = inner.routes.remove(key);
        if prev.is_some() {
            inner.rebuild_host_map();
            self.gen_tx.send_modify(|g| *g += 1);
        }
        prev
    }

    pub async fn get_route(&self, key: &str) -> Option<Route> {
        let inner = self.inner.read().await;
        inner.routes.get(key).cloned()
    }

    pub async fn routes(&self) -> Vec<Route> {
        let inner = self.inner.read().await;
        inner.routes.values().cloned().collect()
    }

    pub async fn set_default_app(&self, app: Option<String>) {
        let mut inner = self.inner.write().await;
        inner.default_app = app;
        self.gen_tx.send_modify(|g| *g += 1);
    }

    pub async fn set_control(&self, host: Option<String>, addr: SocketAddr) {
        let mut inner = self.inner.write().await;
        inner.control_host = host.map(|h| clean_hostname(&h));
        inner.control_addr = Some(addr);
        self.gen_tx.send_modify(|g| *g += 1);
    }

    pub async fn resolve(&self, host: &str) -> Resolution {
        let clean = clean_hostname(host);
        let inner = self.inner.read().await;

        if let Some(key) = inner.host_map.get(&clean) {
            if let Some(route) = inner.routes.get(key) {
                return Resolution::App {
                    key: key.clone(),
                    target: route.target,
                };
            }
        }

        if let (Some(ch), Some(addr)) = (&inner.control_host, inner.control_addr) {
            if ch == &clean {
                return Resolution::Control(addr);
            }
        }

        if is_ip_literal_or_empty(&clean) {
            if let Some(default_app) = &inner.default_app {
                let prod_key = format!("{default_app}:production");
                if let Some(route) = inner.routes.get(&prod_key) {
                    return Resolution::App {
                        key: prod_key,
                        target: route.target,
                    };
                }
                let prev_key = format!("{default_app}:preview");
                if let Some(route) = inner.routes.get(&prev_key) {
                    return Resolution::App {
                        key: prev_key,
                        target: route.target,
                    };
                }
            }
        }

        Resolution::NotFound
    }

    pub async fn all_dns_hosts(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        let mut set = std::collections::BTreeSet::new();
        for route in inner.routes.values() {
            for host in &route.hosts {
                let clean = clean_hostname(host);
                if is_dns_name(&clean) {
                    set.insert(clean);
                }
            }
        }
        if let Some(ch) = &inner.control_host {
            let clean = clean_hostname(ch);
            if is_dns_name(&clean) {
                set.insert(clean);
            }
        }
        set.into_iter().collect()
    }

    pub fn subscribe_generation(&self) -> watch::Receiver<u64> {
        self.gen_tx.subscribe()
    }
}

impl RouterInner {
    fn rebuild_host_map(&mut self) {
        self.host_map.clear();
        for (k, r) in &self.routes {
            for h in &r.hosts {
                let clean = clean_hostname(h);
                if !clean.is_empty() {
                    self.host_map.insert(clean, k.clone());
                }
            }
        }
    }
}
