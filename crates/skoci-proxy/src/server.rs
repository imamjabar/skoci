use crate::router::{clean_hostname, is_dns_name, ProxyRouter, Resolution};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_acme::caches::DirCache;
use rustls_acme::{AcmeConfig, EventOk, ResolvesServerCertAcme};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

static REQ_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_request_id() -> String {
    let count = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("req_{now:x}_{count:x}")
}

type ProxyBody = BoxBody<Bytes, hyper::Error>;

fn full_body(data: impl Into<Bytes>) -> ProxyBody {
    Full::new(data.into())
        .map_err(|e| match e {})
        .boxed()
}

fn empty_body() -> ProxyBody {
    Empty::new()
        .map_err(|e| match e {})
        .boxed()
}

#[derive(Debug, Clone)]
pub struct AcmeSettings {
    pub email: Option<String>,
    pub cache_dir: PathBuf,
    pub staging: bool,
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub http_bind: SocketAddr,
    pub https_bind: SocketAddr,
    pub acme: Option<AcmeSettings>,
}

#[derive(Clone, Default)]
struct CertState {
    issued: bool,
    last_error: Option<String>,
}

struct ProxyStatusInner {
    http_listening: AtomicBool,
    https_listening: AtomicBool,
    tls_enabled: AtomicBool,
    http_addr: RwLock<Option<SocketAddr>>,
    https_addr: RwLock<Option<SocketAddr>>,
    bind_errors: RwLock<Vec<String>>,
    certificates: RwLock<HashMap<String, CertState>>,
}

#[derive(Clone)]
pub struct ProxyStatus {
    inner: Arc<ProxyStatusInner>,
}

impl Default for ProxyStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyStatus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProxyStatusInner {
                http_listening: AtomicBool::new(false),
                https_listening: AtomicBool::new(false),
                tls_enabled: AtomicBool::new(false),
                http_addr: RwLock::new(None),
                https_addr: RwLock::new(None),
                bind_errors: RwLock::new(Vec::new()),
                certificates: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn http_listening(&self) -> bool {
        self.inner.http_listening.load(Ordering::Relaxed)
    }

    pub fn https_listening(&self) -> bool {
        self.inner.https_listening.load(Ordering::Relaxed)
    }

    pub fn tls_enabled(&self) -> bool {
        self.inner.tls_enabled.load(Ordering::Relaxed)
    }

    pub fn certificates(&self) -> Vec<(String, bool, Option<String>)> {
        let certs = self.inner.certificates.read();
        certs
            .iter()
            .map(|(h, s)| (h.clone(), s.issued, s.last_error.clone()))
            .collect()
    }

    pub fn bind_errors(&self) -> Vec<String> {
        self.inner.bind_errors.read().clone()
    }

    pub fn http_addr(&self) -> Option<SocketAddr> {
        *self.inner.http_addr.read()
    }

    pub fn https_addr(&self) -> Option<SocketAddr> {
        *self.inner.https_addr.read()
    }

    pub(crate) fn set_http_listening(&self, val: bool) {
        self.inner.http_listening.store(val, Ordering::Relaxed);
    }

    pub(crate) fn set_https_listening(&self, val: bool) {
        self.inner.https_listening.store(val, Ordering::Relaxed);
    }

    pub(crate) fn set_tls_enabled(&self, val: bool) {
        self.inner.tls_enabled.store(val, Ordering::Relaxed);
    }

    pub(crate) fn set_http_addr(&self, addr: Option<SocketAddr>) {
        *self.inner.http_addr.write() = addr;
    }

    pub(crate) fn set_https_addr(&self, addr: Option<SocketAddr>) {
        *self.inner.https_addr.write() = addr;
    }

    pub(crate) fn add_bind_error(&self, err: String) {
        self.inner.bind_errors.write().push(err);
    }

    pub(crate) fn has_issued_cert(&self, host: &str) -> bool {
        let certs = self.inner.certificates.read();
        certs.get(host).map(|s| s.issued).unwrap_or(false)
    }

    pub(crate) fn init_certificate(&self, host: &str) {
        let mut certs = self.inner.certificates.write();
        certs.entry(host.to_string()).or_default();
    }

    pub(crate) fn set_certificate_status(&self, host: &str, issued: bool, err: Option<String>) {
        let mut certs = self.inner.certificates.write();
        let entry = certs.entry(host.to_string()).or_default();
        entry.issued = issued;
        entry.last_error = err;
    }

    pub(crate) fn remove_certificate(&self, host: &str) {
        let mut certs = self.inner.certificates.write();
        certs.remove(host);
    }
}

#[derive(Debug)]
struct SniCertResolver {
    resolvers: Arc<RwLock<HashMap<String, Arc<ResolvesServerCertAcme>>>>,
}

impl ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let sni = client_hello.server_name()?;
        let clean = clean_hostname(sni);
        let resolver = self.resolvers.read().get(&clean).cloned()?;
        resolver.resolve(client_hello)
    }
}

pub async fn start_proxy(cfg: ProxyConfig, router: ProxyRouter) -> ProxyStatus {
    let status = ProxyStatus::new();

    let mut http_connector = HttpConnector::new();
    http_connector.set_connect_timeout(Some(Duration::from_secs(5)));
    let client: Client<HttpConnector, Incoming> =
        Client::builder(TokioExecutor::new()).build(http_connector);
    let client = Arc::new(client);

    match tokio::net::TcpListener::bind(cfg.http_bind).await {
        Ok(http_listener) => {
            let local_addr = http_listener.local_addr().ok();
            status.set_http_listening(true);
            status.set_http_addr(local_addr);

            let router_clone = router.clone();
            let client_clone = client.clone();
            let status_clone = status.clone();

            tokio::spawn(async move {
                run_http_listener(http_listener, router_clone, client_clone, status_clone).await;
            });
        }
        Err(e) => {
            let msg = format!("Failed to bind HTTP listener on {}: {e}", cfg.http_bind);
            tracing::error!("{msg}");
            status.add_bind_error(msg);
        }
    }

    if let Some(acme_settings) = cfg.acme {
        status.set_tls_enabled(true);

        let resolvers: Arc<RwLock<HashMap<String, Arc<ResolvesServerCertAcme>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let sni_resolver = Arc::new(SniCertResolver {
            resolvers: resolvers.clone(),
        });

        let mut server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("safe TLS protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(sni_resolver);

        server_config.alpn_protocols = vec![
            b"h2".to_vec(),
            b"http/1.1".to_vec(),
            b"acme-tls/1".to_vec(),
        ];
        let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));

        let router_clone = router.clone();
        let status_clone = status.clone();
        let cache_dir = acme_settings.cache_dir;
        let email = acme_settings.email;
        let staging = acme_settings.staging;
        let resolvers_clone = resolvers.clone();

        tokio::spawn(async move {
            run_acme_manager(
                router_clone,
                resolvers_clone,
                status_clone,
                cache_dir,
                email,
                staging,
            )
            .await;
        });

        match tokio::net::TcpListener::bind(cfg.https_bind).await {
            Ok(https_listener) => {
                let local_addr = https_listener.local_addr().ok();
                status.set_https_listening(true);
                status.set_https_addr(local_addr);

                let router_clone = router.clone();
                let client_clone = client.clone();
                let status_clone = status.clone();

                tokio::spawn(async move {
                    run_https_listener(
                        https_listener,
                        tls_acceptor,
                        router_clone,
                        client_clone,
                        status_clone,
                    )
                    .await;
                });
            }
            Err(e) => {
                let msg = format!("Failed to bind HTTPS listener on {}: {e}", cfg.https_bind);
                tracing::error!("{msg}");
                status.add_bind_error(msg);
            }
        }
    }

    status
}

async fn run_acme_manager(
    router: ProxyRouter,
    resolvers: Arc<RwLock<HashMap<String, Arc<ResolvesServerCertAcme>>>>,
    status: ProxyStatus,
    cache_dir: PathBuf,
    email: Option<String>,
    staging: bool,
) {
    let mut gen_rx = router.subscribe_generation();
    let mut active_hosts: HashMap<String, tokio::task::AbortHandle> = HashMap::new();

    loop {
        let hosts = router.all_dns_hosts().await;
        let hosts_set: std::collections::HashSet<String> = hosts.iter().cloned().collect();

        let removed: Vec<String> = active_hosts
            .keys()
            .filter(|h| !hosts_set.contains(*h))
            .cloned()
            .collect();
        for h in removed {
            if let Some(handle) = active_hosts.remove(&h) {
                handle.abort();
            }
            resolvers.write().remove(&h);
            status.remove_certificate(&h);
        }

        for host in hosts {
            if active_hosts.contains_key(&host) {
                continue;
            }

            status.init_certificate(&host);

            let mut acme_config = AcmeConfig::new([host.clone()])
                .cache(DirCache::new(cache_dir.clone()))
                .directory_lets_encrypt(!staging);

            if let Some(em) = &email {
                acme_config = acme_config.contact_push(format!("mailto:{em}"));
            }

            let mut state = acme_config.state();
            let host_resolver = state.resolver();

            resolvers.write().insert(host.clone(), host_resolver);

            let host_clone = host.clone();
            let status_cert = status.clone();

            let join_handle = tokio::spawn(async move {
                use futures_util::StreamExt;
                while let Some(event) = state.next().await {
                    match event {
                        Ok(EventOk::DeployedCachedCert) | Ok(EventOk::DeployedNewCert) => {
                            tracing::info!("TLS certificate active for {host_clone}");
                            status_cert.set_certificate_status(&host_clone, true, None);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let err_msg = e.to_string();
                            tracing::warn!(
                                "Failed to process TLS certificate for {host_clone}: {err_msg}"
                            );
                            status_cert.set_certificate_status(
                                &host_clone,
                                false,
                                Some(err_msg),
                            );
                        }
                    }
                }
            });

            active_hosts.insert(host, join_handle.abort_handle());
        }

        if gen_rx.changed().await.is_err() {
            break;
        }
    }
}

async fn run_http_listener(
    listener: tokio::net::TcpListener,
    router: ProxyRouter,
    client: Arc<Client<HttpConnector, Incoming>>,
    status: ProxyStatus,
) {
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!("HTTP accept error: {e}");
                continue;
            }
        };

        let io = TokioIo::new(stream);
        let router_clone = router.clone();
        let client_clone = client.clone();
        let status_clone = status.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let router_inner = router_clone.clone();
                let client_inner = client_clone.clone();
                let status_inner = status_clone.clone();
                async move {
                    handle_proxy_request(
                        req,
                        peer_addr.ip(),
                        "http",
                        router_inner,
                        client_inner,
                        status_inner,
                    )
                    .await
                }
            });

            let builder = Builder::new(TokioExecutor::new());
            if let Err(e) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::debug!("HTTP connection closed: {e}");
            }
        });
    }
}

async fn run_https_listener(
    listener: tokio::net::TcpListener,
    tls_acceptor: tokio_rustls::TlsAcceptor,
    router: ProxyRouter,
    client: Arc<Client<HttpConnector, Incoming>>,
    status: ProxyStatus,
) {
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!("HTTPS accept error: {e}");
                continue;
            }
        };

        let tls_acceptor = tls_acceptor.clone();
        let router_clone = router.clone();
        let client_clone = client.clone();
        let status_clone = status.clone();

        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("TLS handshake failed from {peer_addr}: {e}");
                    return;
                }
            };

            if let Some(b"acme-tls/1") = tls_stream.get_ref().1.alpn_protocol() {
                return;
            }

            let io = TokioIo::new(tls_stream);
            let service = service_fn(move |req: Request<Incoming>| {
                let router_inner = router_clone.clone();
                let client_inner = client_clone.clone();
                let status_inner = status_clone.clone();
                async move {
                    handle_proxy_request(
                        req,
                        peer_addr.ip(),
                        "https",
                        router_inner,
                        client_inner,
                        status_inner,
                    )
                    .await
                }
            });

            let builder = Builder::new(TokioExecutor::new());
            if let Err(e) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::debug!("HTTPS connection closed: {e}");
            }
        });
    }
}

fn is_hop_by_hop(name: &hyper::header::HeaderName, is_upgrade: bool) -> bool {
    let s = name.as_str();
    if is_upgrade && (s.eq_ignore_ascii_case("connection") || s.eq_ignore_ascii_case("upgrade")) {
        return false;
    }
    s.eq_ignore_ascii_case("connection")
        || s.eq_ignore_ascii_case("keep-alive")
        || s.eq_ignore_ascii_case("proxy-authenticate")
        || s.eq_ignore_ascii_case("proxy-authorization")
        || s.eq_ignore_ascii_case("proxy-connection")
        || s.eq_ignore_ascii_case("te")
        || s.eq_ignore_ascii_case("trailer")
        || s.eq_ignore_ascii_case("transfer-encoding")
        || s.eq_ignore_ascii_case("upgrade")
}

async fn handle_proxy_request(
    mut req: Request<Incoming>,
    peer_ip: IpAddr,
    proto: &'static str,
    router: ProxyRouter,
    client: Arc<Client<HttpConnector, Incoming>>,
    status: ProxyStatus,
) -> Result<Response<ProxyBody>, std::convert::Infallible> {
    let host_hdr = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host = clean_hostname(&host_hdr);

    if proto == "http" && is_dns_name(&host) && status.has_issued_cert(&host) {
        let pq = req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("/");
        let location = format!("https://{host_hdr}{pq}");
        return Ok(Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header(hyper::header::LOCATION, location)
            .header(hyper::header::CONTENT_LENGTH, "0")
            .body(empty_body())
            .unwrap());
    }

    let resolution = router.resolve(&host).await;

    if proto == "http" {
        if let Resolution::Control(_) = resolution {
            if status.tls_enabled() {
                let pq = req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("/");
                let location = format!("https://{host_hdr}{pq}");
                return Ok(Response::builder()
                    .status(StatusCode::PERMANENT_REDIRECT)
                    .header(hyper::header::LOCATION, location)
                    .header(hyper::header::CONTENT_LENGTH, "0")
                    .body(empty_body())
                    .unwrap());
            } else {
                let msg = "Public HTTP access to Skoci API is forbidden. Use HTTPS (--tls acme) or an SSH tunnel: ssh -L 8787:127.0.0.1:8787 user@server\n";
                return Ok(Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                    .body(full_body(msg))
                    .unwrap());
            }
        }
    }

    let (target, target_app) = match resolution {
        Resolution::App { key, target } => {
            let app = key.split(':').next().unwrap_or("app").to_string();
            (target, Some(app))
        }
        Resolution::Control(addr) => (addr, None),
        Resolution::NotFound => {
            let body = format!(
                "404 Not Found: Domain '{host}' is not registered in Skoci.\n\
                 Hint: add the domain to skoci.yaml then run skoci deploy\n"
            );
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(full_body(body))
                .unwrap());
        }
    };

    let is_upgrade = req
        .headers()
        .get(hyper::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("upgrade"))
        .unwrap_or(false)
        && req.headers().contains_key(hyper::header::UPGRADE);

    let client_upgrade = if is_upgrade {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };

    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(generate_request_id);

    let incoming_xfwd = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let x_forwarded_for = match incoming_xfwd {
        Some(existing) => format!("{existing}, {peer_ip}"),
        None => peer_ip.to_string(),
    };

    let pq = req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("/");
    let upstream_uri: hyper::Uri = format!("http://{target}{pq}").parse().unwrap();
    let method = req.method().clone();

    let (parts, body) = req.into_parts();
    let mut fwd_builder = Request::builder().method(method).uri(upstream_uri);
    let fwd_headers = fwd_builder.headers_mut().unwrap();

    for (k, v) in parts.headers {
        if let Some(key) = k {
            let name_str = key.as_str();
            if is_hop_by_hop(&key, is_upgrade)
                || name_str.eq_ignore_ascii_case("host")
                || name_str.eq_ignore_ascii_case("x-forwarded-for")
                || name_str.eq_ignore_ascii_case("x-forwarded-proto")
                || name_str.eq_ignore_ascii_case("x-forwarded-host")
                || name_str.eq_ignore_ascii_case("x-request-id")
            {
                continue;
            }
            fwd_headers.insert(key, v);
        }
    }

    let host_val = if host_hdr.is_empty() { "localhost" } else { &host_hdr };
    if let Ok(hv) = hyper::header::HeaderValue::from_str(host_val) {
        fwd_headers.insert(hyper::header::HOST, hv);
    }
    if let Ok(xf) = hyper::header::HeaderValue::from_str(&x_forwarded_for) {
        fwd_headers.insert(
            hyper::header::HeaderName::from_static("x-forwarded-for"),
            xf,
        );
    }
    fwd_headers.insert(
        hyper::header::HeaderName::from_static("x-forwarded-proto"),
        hyper::header::HeaderValue::from_static(proto),
    );
    if let Ok(xh) = hyper::header::HeaderValue::from_str(host_val) {
        fwd_headers.insert(
            hyper::header::HeaderName::from_static("x-forwarded-host"),
            xh,
        );
    }
    if let Ok(xr) = hyper::header::HeaderValue::from_str(&request_id) {
        fwd_headers.insert(hyper::header::HeaderName::from_static("x-request-id"), xr);
    }

    let fwd_req = fwd_builder.body(body).unwrap();

    let mut upstream_resp = match client.request(fwd_req).await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!("Failed to connect to upstream {target}: {err}");
            let app_hint = match &target_app {
                Some(app) => format!(" 'skoci logs {app}'"),
                None => " 'skoci logs'".to_string(),
            };
            let body = format!(
                "502 Bad Gateway: Failed to connect to upstream {target}.\n\
                 Hint: run{app_hint} to view container logs.\n"
            );
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header("x-request-id", &request_id)
                .body(full_body(body))
                .unwrap());
        }
    };

    let is_101 = is_upgrade && upstream_resp.status() == StatusCode::SWITCHING_PROTOCOLS;

    let server_upgrade = if is_101 {
        Some(hyper::upgrade::on(&mut upstream_resp))
    } else {
        None
    };

    let (resp_parts, resp_body) = upstream_resp.into_parts();
    let mut resp_builder = Response::builder().status(resp_parts.status);
    let resp_headers = resp_builder.headers_mut().unwrap();

    for (k, v) in resp_parts.headers {
        if let Some(key) = k {
            if is_hop_by_hop(&key, is_101) {
                continue;
            }
            resp_headers.insert(key, v);
        }
    }
    if let Ok(xr) = hyper::header::HeaderValue::from_str(&request_id) {
        resp_headers.insert(hyper::header::HeaderName::from_static("x-request-id"), xr);
    }

    if is_101 {
        if let (Some(client_upgrade), Some(server_upgrade)) = (client_upgrade, server_upgrade) {
            tokio::spawn(async move {
                match tokio::join!(client_upgrade, server_upgrade) {
                    (Ok(client_io), Ok(server_io)) => {
                        let mut client_io = TokioIo::new(client_io);
                        let mut server_io = TokioIo::new(server_io);
                        if let Err(e) =
                            tokio::io::copy_bidirectional(&mut client_io, &mut server_io).await
                        {
                            tracing::debug!("Upgrade bidirectional copy error: {e}");
                        }
                    }
                    (Err(e), _) => tracing::error!("Client upgrade error: {e}"),
                    (_, Err(e)) => tracing::error!("Server upgrade error: {e}"),
                }
            });
        }
        Ok(resp_builder.body(empty_body()).unwrap())
    } else {
        Ok(resp_builder.body(resp_body.boxed()).unwrap())
    }
}
