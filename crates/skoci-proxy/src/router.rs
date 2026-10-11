use skoci_api::error::{SkociError, SkociResult};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
    routes: BTreeMap<String, Route>,
    host_map: BTreeMap<String, String>,
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

pub fn validate_authority_syntax(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return true;
    }
    if raw.contains('@') {
        return false;
    }
    if raw.starts_with('[') {
        let end = match raw.find(']') {
            Some(idx) => idx,
            None => return false,
        };
        let ip_str = &raw[1..end];
        if ip_str.parse::<Ipv6Addr>().is_err() {
            return false;
        }
        let rest = &raw[end + 1..];
        if rest.is_empty() {
            return true;
        }
        if let Some(port_str) = rest.strip_prefix(':') {
            if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            matches!(port_str.parse::<u16>(), Ok(p) if p > 0)
        } else {
            false
        }
    } else {
        let (host_str, port_opt) = match raw.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (raw, None),
        };
        if let Some(port_str) = port_opt {
            if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            if !matches!(port_str.parse::<u16>(), Ok(p) if p > 0) {
                return false;
            }
        }
        if host_str.parse::<Ipv4Addr>().is_ok() {
            return true;
        }
        if host_str.is_empty() || host_str.len() > 253 {
            return false;
        }
        for label in host_str.split('.') {
            if label.is_empty() || label.len() > 63 {
                return false;
            }
            if label.starts_with('-') || label.ends_with('-') {
                return false;
            }
            if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    pub clean_host: String,
    pub forward_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingAuthorityError {
    BadRequest(&'static str),
    MisdirectedRequest(&'static str),
}

pub fn decide_routing_authority(
    uri_authority: Option<&str>,
    host_header: Option<&str>,
    is_https: bool,
    sni: Option<&str>,
) -> Result<RoutingDecision, RoutingAuthorityError> {
    let raw_authority = match (uri_authority, host_header) {
        (Some(u), Some(h)) => {
            if !u.eq_ignore_ascii_case(h) {
                return Err(RoutingAuthorityError::BadRequest(
                    "URI authority and Host header do not match",
                ));
            }
            u
        }
        (Some(u), None) => u,
        (None, Some(h)) => h,
        (None, None) => "",
    };

    if !validate_authority_syntax(raw_authority) {
        return Err(RoutingAuthorityError::BadRequest(
            "Invalid host or authority syntax",
        ));
    }

    let clean = clean_hostname(raw_authority);

    if is_https {
        if is_ip_literal_or_empty(&clean) {
            return Err(RoutingAuthorityError::MisdirectedRequest(
                "Empty or IP authority forbidden on HTTPS",
            ));
        }
        let sni_host = match sni {
            Some(s) if !s.trim().is_empty() => clean_hostname(s),
            _ => {
                return Err(RoutingAuthorityError::MisdirectedRequest(
                    "Missing TLS SNI on HTTPS request",
                ));
            }
        };
        if !clean.eq_ignore_ascii_case(&sni_host) {
            return Err(RoutingAuthorityError::MisdirectedRequest(
                "Authority does not match TLS SNI",
            ));
        }
    }

    let forward_host = if raw_authority.is_empty() {
        "localhost".to_string()
    } else {
        raw_authority.to_string()
    };

    Ok(RoutingDecision {
        clean_host: clean,
        forward_host,
    })
}

impl ProxyRouter {
    pub fn new() -> Self {
        let (gen_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(RwLock::new(RouterInner {
                routes: BTreeMap::new(),
                host_map: BTreeMap::new(),
                default_app: None,
                control_host: None,
                control_addr: None,
            })),
            gen_tx,
        }
    }

    pub async fn upsert_route(&self, route: Route) -> SkociResult<Option<Route>> {
        let key = route.key();
        let mut inner = self.inner.write().await;

        for h in &route.hosts {
            let clean = clean_hostname(h);
            if clean.is_empty() {
                continue;
            }
            if let Some(ch) = &inner.control_host {
                if ch == &clean {
                    return Err(SkociError::conflict(
                        format!("Host '{clean}' is reserved for the control plane"),
                        "Use a different domain name",
                    ));
                }
            }
            if let Some(owner) = inner.host_map.get(&clean) {
                if owner != &key {
                    return Err(SkociError::conflict(
                        format!("Host '{clean}' is already mapped to route '{owner}'"),
                        "Use a different domain name or remove the conflicting route first",
                    ));
                }
            }
        }

        let prev = inner.routes.insert(key, route);
        inner.rebuild_host_map();
        self.gen_tx.send_modify(|g| *g += 1);
        Ok(prev)
    }

    pub async fn host_owner(&self, host: &str) -> Option<String> {
        let clean = clean_hostname(host);
        let inner = self.inner.read().await;
        inner.host_map.get(&clean).cloned()
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
        inner.rebuild_host_map();
        self.gen_tx.send_modify(|g| *g += 1);
    }

    pub async fn resolve(&self, host: &str) -> Resolution {
        let clean = clean_hostname(host);
        let inner = self.inner.read().await;

        if let (Some(ch), Some(addr)) = (&inner.control_host, inner.control_addr) {
            if ch == &clean {
                return Resolution::Control(addr);
            }
        }

        if let Some(key) = inner.host_map.get(&clean) {
            if let Some(route) = inner.routes.get(key) {
                return Resolution::App {
                    key: key.clone(),
                    target: route.target,
                };
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
                if clean.is_empty() {
                    continue;
                }
                if let Some(ch) = &self.control_host {
                    if ch == &clean {
                        continue;
                    }
                }
                self.host_map.entry(clean).or_insert_with(|| k.clone());
            }
        }
    }
}
