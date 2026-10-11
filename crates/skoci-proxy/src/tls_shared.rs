use parking_lot::RwLock;
use rustls::sign::CertifiedKey;
use std::collections::HashMap;
use std::sync::Arc;

/// Prefix of ACME http-01 challenge paths answered by the HTTP listener (port 80) for any Host.
pub const HTTP01_PREFIX: &str = "/.well-known/acme-challenge/";

/// Pending ACME http-01 tokens → key authorizations. Filled by the control-plane IP-certificate manager,
/// answered by the proxy HTTP listener before routing.
#[derive(Clone, Default, Debug)]
pub struct HttpChallenges(Arc<RwLock<HashMap<String, String>>>);

impl HttpChallenges {
    pub fn insert(&self, token: impl Into<String>, key_authorization: impl Into<String>) {
        self.0.write().insert(token.into(), key_authorization.into());
    }

    pub fn remove(&self, token: &str) {
        self.0.write().remove(token);
    }

    pub fn get(&self, token: &str) -> Option<String> {
        self.0.read().get(token).cloned()
    }
}

/// Hot-swappable certificate. Used for TLS handshakes without SNI (`https://<public-ip>`) on the proxy
/// and for every handshake on the control endpoint. Always holds the control key, so the SPKI pin is stable.
#[derive(Clone, Default, Debug)]
pub struct CertSlot(Arc<RwLock<Option<Arc<CertifiedKey>>>>);

impl CertSlot {
    pub fn set(&self, cert: Arc<CertifiedKey>) {
        *self.0.write() = Some(cert);
    }

    pub fn get(&self) -> Option<Arc<CertifiedKey>> {
        self.0.read().clone()
    }
}
