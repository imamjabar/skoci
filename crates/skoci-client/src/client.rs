use futures_util::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{
    ApproveRequest, DbBackupRequest, DbRestoreRequest, DefaultAppRequest, DeployRequest,
    DeploymentsQuery, DomainCheckRequest, DomainSetPrimaryRequest, LogsQuery, PromoteRequest,
    RollbackRequest, SecretSetRequest, TokenCreateRequest,
};
use skoci_api::responses::{
    AppDeleteResponse, AppSummary, ApprovalInfo, ApproveResponse, AuditEntry, DbBackupListResponse,
    DbBackupResponse, DbRestoreResponse, DefaultAppResponse, DeployAccepted, DeployEvent,
    DeployStage, DeploymentInfo, DoctorResponse, DomainCheckResponse, DomainSetPrimaryResponse,
    LogsResponse, PromoteResponse, SecretListResponse, StatusResponse, TokenCreateResponse,
    TokenListResponse, TokenRevokeResponse,
};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::bytes::Bytes;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};

#[derive(Debug, Clone)]
struct PinMismatch {
    server: String,
    expected: String,
    got: String,
}

#[derive(Debug)]
struct SpkiPinVerifier {
    server: String,
    expected_pin: String,
    mismatch_slot: Arc<parking_lot::Mutex<Option<PinMismatch>>>,
    supported_algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

impl ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let (_, cert) = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| RustlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;

        let spki_der = cert.tbs_certificate.subject_pki.raw;
        let got = skoci_api::models::spki_pin_from_der(spki_der);

        if !constant_time_eq(got.as_bytes(), self.expected_pin.as_bytes()) {
            *self.mismatch_slot.lock() = Some(PinMismatch {
                server: self.server.clone(),
                expected: self.expected_pin.clone(),
                got: got.clone(),
            });
            return Err(RustlsError::General(format!(
                "SPKI pin mismatch: expected {}, got {}",
                self.expected_pin, got
            )));
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

#[derive(Clone)]
pub struct SkociClient {
    base_url: String,
    token: Option<String>,
    pin: Option<String>,
    client: reqwest::Client,
    pin_mismatch_slot: Option<Arc<parking_lot::Mutex<Option<PinMismatch>>>>,
}

impl std::fmt::Debug for SkociClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkociClient")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
            .field("pin", &self.pin)
            .finish()
    }
}

impl SkociClient {
    pub fn new(
        base_url: impl Into<String>,
        token: Option<String>,
        pin: Option<String>,
    ) -> SkociResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();

        let normalized_pin = match pin.as_deref() {
            Some(p) if !p.trim().is_empty() => match skoci_api::models::normalize_spki_pin(p) {
                Some(np) => Some(np),
                None => {
                    return Err(SkociError::bad_request(
                            format!("Malformed SPKI pin '{p}': expected sha256:<64 hex> or bare 64-char hex"),
                            "Check the pin format or retrieve a fresh pin with 'skoci server fingerprint'",
                        ));
                }
            },
            _ => None,
        };

        let parsed_url = reqwest::Url::parse(&base_url).map_err(|e| {
            SkociError::bad_request(
                format!("Invalid server URL '{base_url}': {e}"),
                "Provide a valid URL with scheme (e.g., https://1.2.3.4:8443 or http://localhost:8787)",
            )
        })?;

        let host_str = parsed_url.host_str().unwrap_or("").to_string();
        let is_ip_literal = host_str.parse::<std::net::IpAddr>().is_ok()
            || host_str
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok();

        if parsed_url.scheme() == "https" && is_ip_literal && normalized_pin.is_none() {
            return Err(SkociError::pin_required(&host_str));
        }

        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
        let pin_mismatch_slot = if let Some(expected_pin) = &normalized_pin {
            let mismatch_slot = Arc::new(parking_lot::Mutex::new(None));
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let supported_algs = provider.signature_verification_algorithms;
            let verifier = Arc::new(SpkiPinVerifier {
                server: host_str.clone(),
                expected_pin: expected_pin.clone(),
                mismatch_slot: Arc::clone(&mismatch_slot),
                supported_algs,
            });

            let client_config: rustls::ClientConfig =
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(|e| {
                        SkociError::internal(
                            format!("Failed to configure TLS protocol versions: {e}"),
                            "Check TLS configuration",
                        )
                    })?
                    .dangerous()
                    .with_custom_certificate_verifier(verifier)
                    .with_no_client_auth();

            builder = builder.use_preconfigured_tls(client_config);
            Some(mismatch_slot)
        } else {
            None
        };

        let client = builder.build().map_err(|e| {
            SkociError::internal(
                format!("Failed to build HTTP client: {e}"),
                "Check client configuration and system network stack",
            )
        })?;

        Ok(Self {
            base_url,
            token,
            pin: normalized_pin,
            client,
            pin_mismatch_slot,
        })
    }

    fn map_reqwest_err(
        &self,
        e: &reqwest::Error,
        default_msg: String,
        default_fix: impl Into<String>,
    ) -> SkociError {
        if let Some(slot) = &self.pin_mismatch_slot {
            if let Some(m) = slot.lock().take() {
                return SkociError::pin_mismatch(&m.server, &m.expected, &m.got);
            }
        }

        let err_str = e.to_string();
        if let Some(pos) = err_str.find("SPKI pin mismatch: expected ") {
            let rest = &err_str[pos + "SPKI pin mismatch: expected ".len()..];
            if let Some(sep) = rest.find(", got ") {
                let exp = &rest[..sep];
                let got_rest = &rest[sep + ", got ".len()..];
                let got = got_rest
                    .split(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '"')
                    .next()
                    .unwrap_or(got_rest);
                let server = self.server_host();
                return SkociError::pin_mismatch(&server, exp, got);
            }
        }

        SkociError::internal(default_msg, default_fix.into())
    }

    fn server_host(&self) -> String {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| self.base_url.clone())
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn pin(&self) -> Option<&str> {
        self.pin.as_deref()
    }

    fn auth(&self, mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            rb = rb.header("Authorization", format!("Bearer {token}"));
        }
        rb
    }

    async fn send<T: DeserializeOwned>(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> SkociResult<ApiResponse<T>> {
        let resp = rb.send().await.map_err(|e| {
            self.map_reqwest_err(
                &e,
                format!("Failed to connect to Skoci server: {e}"),
                "Check whether the Skoci server is running and the server URL is correct (SKOCI_SERVER_URL)",
            )
        })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            SkociError::internal(
                format!("Failed to read response from Skoci server: {e}"),
                "Check network connection and Skoci server",
            )
        })?;

        match serde_json::from_str::<ApiResponse<T>>(&text) {
            Ok(parsed) => Ok(parsed),
            Err(_) => {
                let snippet: String = text.chars().take(200).collect();
                Err(SkociError::internal(
                    format!(
                        "Response is not valid JSON format (HTTP {}): {}",
                        status.as_u16(),
                        snippet
                    ),
                    "Check whether SKOCI_SERVER_URL points to the correct Skoci server and not another service",
                ))
            }
        }
    }

    pub async fn deploy_upload(
        &self,
        path: impl AsRef<Path>,
        req: &DeployRequest,
        progress: impl Fn(u64, u64) + Send + Sync + 'static,
    ) -> SkociResult<ApiResponse<DeployAccepted>> {
        let path = path.as_ref();
        let file = tokio::fs::File::open(path).await.map_err(|e| {
            SkociError::bad_request(
                format!("Failed to open context file {}: {e}", path.display()),
                "Ensure context archive file exists and is readable",
            )
        })?;
        let total_bytes = file
            .metadata()
            .await
            .map_err(|e| {
                SkociError::internal(
                    format!("Failed to read file metadata: {e}"),
                    "Try deploying again",
                )
            })?
            .len();

        let sent = Arc::new(AtomicU64::new(0));
        let progress = Arc::new(progress);

        let stream = tokio_util::io::ReaderStream::new(file).map(move |res| {
            if let Ok(bytes) = &res {
                let cur = sent.fetch_add(bytes.len() as u64, Ordering::SeqCst) + bytes.len() as u64;
                progress(cur, total_bytes);
            }
            res
        });

        let body = reqwest::Body::wrap_stream(stream);
        let mut url = format!(
            "{}/api/deploy?app={}&env={}",
            self.base_url, req.app, req.env
        );
        if let Some(commit) = &req.commit {
            url.push_str(&format!("&commit={}", commit));
        }
        let rb = self
            .client
            .post(&url)
            .header("Content-Type", "application/gzip")
            .body(body);

        self.send(self.auth(rb)).await
    }

    pub fn follow_events(
        &self,
        id: &str,
    ) -> impl Stream<Item = SkociResult<DeployEvent>> + Unpin + 'static {
        let url = format!("{}/api/deployments/{}/events", self.base_url, id);
        let rb = self.client.get(&url).header("Accept", "text/event-stream");
        let rb = self.auth(rb);
        let id_str = id.to_string();

        let this = self.clone();
        let init = async move {
            let resp = rb.send().await.map_err(|e| {
                this.map_reqwest_err(
                    &e,
                    format!("Failed to connect to event stream: {e}"),
                    format!("Use `skoci watch {id_str}` to retry"),
                )
            })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(SkociError::internal(
                    format!("Server responded with HTTP error {status}: {text}"),
                    format!("Use `skoci watch {id_str}` to retry"),
                ));
            }
            Ok(resp.bytes_stream())
        };

        let deployment_id = id.to_string();
        Box::pin(futures_util::stream::once(init).flat_map(move |res| {
            let id = deployment_id.clone();
            match res {
                Ok(stream) => parse_sse_stream(stream, id).boxed(),
                Err(err) => futures_util::stream::once(async move { Err(err) }).boxed(),
            }
        }))
    }

    pub async fn deployment(&self, id: &str) -> SkociResult<ApiResponse<DeploymentInfo>> {
        let url = format!("{}/api/deployments/{}", self.base_url, id);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn deployments(
        &self,
        query: DeploymentsQuery,
    ) -> SkociResult<ApiResponse<Vec<DeploymentInfo>>> {
        let url = format!("{}/api/deployments", self.base_url);
        let rb = self.client.get(&url).query(&query);
        self.send(self.auth(rb)).await
    }

    pub async fn apps(&self) -> SkociResult<ApiResponse<Vec<AppSummary>>> {
        let url = format!("{}/api/apps", self.base_url);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn status(&self, app: &str) -> SkociResult<ApiResponse<StatusResponse>> {
        let url = format!("{}/api/status/{}", self.base_url, app);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn logs(
        &self,
        app: &str,
        query: LogsQuery,
    ) -> SkociResult<ApiResponse<LogsResponse>> {
        let url = format!("{}/api/logs/{}", self.base_url, app);
        let rb = self.client.get(&url).query(&query);
        self.send(self.auth(rb)).await
    }

    pub async fn secret_set(&self, req: SecretSetRequest) -> SkociResult<ApiResponse<()>> {
        let url = format!("{}/api/secret", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn secret_list(&self, app: &str) -> SkociResult<ApiResponse<SecretListResponse>> {
        let url = format!("{}/api/secret/{}", self.base_url, app);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn promote(&self, req: PromoteRequest) -> SkociResult<ApiResponse<PromoteResponse>> {
        let url = format!("{}/api/promote", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn rollback(&self, req: RollbackRequest) -> SkociResult<ApiResponse<DeployAccepted>> {
        let url = format!("{}/api/rollback", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn approvals(&self) -> SkociResult<ApiResponse<Vec<ApprovalInfo>>> {
        let url = format!("{}/api/approvals", self.base_url);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn approve(&self, req: ApproveRequest) -> SkociResult<ApiResponse<ApproveResponse>> {
        let url = format!("{}/api/approve", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn reject(&self, req: ApproveRequest) -> SkociResult<ApiResponse<ApproveResponse>> {
        let url = format!("{}/api/reject", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn db_backup(
        &self,
        req: DbBackupRequest,
    ) -> SkociResult<ApiResponse<DbBackupResponse>> {
        let url = format!("{}/api/db/backup", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn db_backups(&self, app: &str) -> SkociResult<ApiResponse<DbBackupListResponse>> {
        let url = format!("{}/api/db/backups/{}", self.base_url, app);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn db_restore(
        &self,
        req: DbRestoreRequest,
    ) -> SkociResult<ApiResponse<DbRestoreResponse>> {
        let url = format!("{}/api/db/restore", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn app_delete(&self, app: &str) -> SkociResult<ApiResponse<AppDeleteResponse>> {
        let url = format!("{}/api/apps/{}", self.base_url, app);
        self.send(self.auth(self.client.delete(&url))).await
    }

    pub async fn default_app_get(&self) -> SkociResult<ApiResponse<DefaultAppResponse>> {
        let url = format!("{}/api/default-app", self.base_url);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn default_app_put(
        &self,
        req: DefaultAppRequest,
    ) -> SkociResult<ApiResponse<DefaultAppResponse>> {
        let url = format!("{}/api/default-app", self.base_url);
        let rb = self.client.put(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn token_create(
        &self,
        req: TokenCreateRequest,
    ) -> SkociResult<ApiResponse<TokenCreateResponse>> {
        let url = format!("{}/api/tokens", self.base_url);
        let rb = self.client.post(&url).json(&req);
        self.send(self.auth(rb)).await
    }

    pub async fn token_list(&self, app: &str) -> SkociResult<ApiResponse<TokenListResponse>> {
        let url = format!("{}/api/tokens/{}", self.base_url, app);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn token_revoke(
        &self,
        app: &str,
        name: &str,
    ) -> SkociResult<ApiResponse<TokenRevokeResponse>> {
        let url = format!("{}/api/tokens/{}/{}", self.base_url, app, name);
        self.send(self.auth(self.client.delete(&url))).await
    }

    pub async fn audits(&self) -> SkociResult<ApiResponse<Vec<AuditEntry>>> {
        let url = format!("{}/api/audits", self.base_url);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn doctor(&self) -> SkociResult<ApiResponse<DoctorResponse>> {
        let url = format!("{}/api/doctor", self.base_url);
        self.send(self.auth(self.client.get(&url))).await
    }

    pub async fn domain_check(&self, host: &str) -> SkociResult<ApiResponse<DomainCheckResponse>> {
        let url = format!("{}/api/domains/check", self.base_url);
        let body = DomainCheckRequest {
            host: host.to_string(),
        };
        let rb = self.auth(self.client.post(&url).json(&body));
        self.send(rb).await
    }

    pub async fn domain_set_primary(
        &self,
        host: &str,
        force: bool,
    ) -> SkociResult<ApiResponse<DomainSetPrimaryResponse>> {
        let url = format!("{}/api/domains/primary", self.base_url);
        let body = DomainSetPrimaryRequest {
            host: host.to_string(),
            force,
        };
        let rb = self.auth(self.client.post(&url).json(&body));
        self.send(rb).await
    }
}

pub fn parse_sse_stream<S, E>(
    stream: S,
    deployment_id: String,
) -> impl Stream<Item = SkociResult<DeployEvent>> + Unpin
where
    S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
    E: std::fmt::Display + 'static,
{
    struct State<S> {
        stream: S,
        buffer: String,
        data_lines: Vec<String>,
        deployment_id: String,
        saw_done: bool,
        eof_reached: bool,
        terminated: bool,
    }

    let state = State {
        stream,
        buffer: String::new(),
        data_lines: Vec::new(),
        deployment_id,
        saw_done: false,
        eof_reached: false,
        terminated: false,
    };

    Box::pin(futures_util::stream::unfold(state, |mut state| {
        Box::pin(async move {
            if state.terminated {
                return None;
            }

            loop {
                if let Some(pos) = state.buffer.find('\n') {
                    let line = state.buffer[..pos].trim_end_matches('\r').to_string();
                    state.buffer.drain(..=pos);

                    if line.is_empty() {
                        if !state.data_lines.is_empty() {
                            let payload = state.data_lines.join("\n");
                            state.data_lines.clear();
                            match serde_json::from_str::<DeployEvent>(&payload) {
                                Ok(event) => {
                                    if event.stage == DeployStage::Done {
                                        state.saw_done = true;
                                        state.terminated = true;
                                    }
                                    return Some((Ok(event), state));
                                }
                                Err(e) => {
                                    state.terminated = true;
                                    return Some((
                                        Err(SkociError::internal(
                                            format!("Failed to parse SSE event: {e}"),
                                            "Check SSE data format from server",
                                        )),
                                        state,
                                    ));
                                }
                            }
                        }
                    } else if line.starts_with(':') {
                        // SSE comment / keepalive line
                        continue;
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        let data_str = rest.strip_prefix(' ').unwrap_or(rest);
                        state.data_lines.push(data_str.to_string());
                    }
                    continue;
                }

                if state.eof_reached {
                    state.terminated = true;
                    if !state.saw_done {
                        return Some((
                            Err(SkociError::internal(
                                "Event connection closed before deployment completed",
                                format!("Use `skoci watch {}` to reconnect", state.deployment_id),
                            )),
                            state,
                        ));
                    }
                    return None;
                }

                match state.stream.next().await {
                    Some(Ok(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        state.buffer.push_str(&text);
                    }
                    Some(Err(e)) => {
                        state.terminated = true;
                        return Some((
                            Err(SkociError::internal(
                                format!("SSE connection closed with error: {e}"),
                                format!("Use `skoci watch {}` to reconnect", state.deployment_id),
                            )),
                            state,
                        ));
                    }
                    None => {
                        state.eof_reached = true;
                        // Flush trailing line if buffer wasn't terminated with newline
                        if !state.buffer.trim().is_empty() {
                            let line = state.buffer.trim_end_matches('\r').to_string();
                            state.buffer.clear();
                            if let Some(rest) = line.strip_prefix("data:") {
                                let data_str = rest.strip_prefix(' ').unwrap_or(rest);
                                state.data_lines.push(data_str.to_string());
                            }
                        }
                        if !state.data_lines.is_empty() {
                            let payload = state.data_lines.join("\n");
                            state.data_lines.clear();
                            if let Ok(event) = serde_json::from_str::<DeployEvent>(&payload) {
                                if event.stage == DeployStage::Done {
                                    state.saw_done = true;
                                    state.terminated = true;
                                }
                                return Some((Ok(event), state));
                            }
                        }
                        if !state.saw_done {
                            state.terminated = true;
                            return Some((
                                Err(SkociError::internal(
                                    "Event connection closed before deployment completed",
                                    format!(
                                        "Use `skoci watch {}` to reconnect",
                                        state.deployment_id
                                    ),
                                )),
                                state,
                            ));
                        }
                        return None;
                    }
                }
            }
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use skoci_api::responses::StageStatus;

    #[tokio::test]
    async fn test_parse_sse_stream_done() {
        let chunk1 = "data: {\"seq\":1,\"ts\":\"2026-10-11T00:00:00Z\",\"stage\":\"build\",\"status\":\"running\",\"message\":\"Building\",\"progress\":20}\n\n";
        let chunk2 = ": keepalive\n\r\ndata: {\"seq\":2,\"ts\":\"2026-10-11T00:00:01Z\",\"stage\":\"done\",\"status\":\"ok\",\"message\":\"Done\",\"progress\":100}\r\n\r\n";

        let stream = stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from(chunk1)),
            Ok::<_, std::io::Error>(Bytes::from(chunk2)),
        ]);

        let mut sse = parse_sse_stream(stream, "dep_123".to_string());
        let ev1 = sse.next().await.unwrap().unwrap();
        assert_eq!(ev1.seq, 1);
        assert_eq!(ev1.stage, DeployStage::Build);

        let ev2 = sse.next().await.unwrap().unwrap();
        assert_eq!(ev2.seq, 2);
        assert_eq!(ev2.stage, DeployStage::Done);

        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn test_parse_sse_split_chunk_and_multiline() {
        // Chunk split in middle of JSON and data: multi-line
        let chunk1 = "data: {\"seq\":1,\"ts\":\"2026-10-11T00:00:00Z\",\n";
        let chunk2 = "data: \"stage\":\"build\",\"status\":\"log\",\"message\":\"line 1\\nline 2\",\"progress\":30}\n\n";
        let chunk3 = "data: {\"seq\":2,\"ts\":\"2026-10-11T00:00:01Z\",\"stage\":\"done\",\"status\":\"ok\",\"message\":\"Complete\",\"progress\":100}\n\n";

        let stream = stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from(chunk1)),
            Ok::<_, std::io::Error>(Bytes::from(chunk2)),
            Ok::<_, std::io::Error>(Bytes::from(chunk3)),
        ]);

        let mut sse = parse_sse_stream(stream, "dep_split".to_string());
        let ev1 = sse.next().await.unwrap().unwrap();
        assert_eq!(ev1.seq, 1);
        assert_eq!(ev1.status, StageStatus::Log);

        let ev2 = sse.next().await.unwrap().unwrap();
        assert_eq!(ev2.seq, 2);
        assert_eq!(ev2.stage, DeployStage::Done);
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn test_parse_sse_eof_before_done() {
        let chunk = "data: {\"seq\":1,\"ts\":\"2026-10-11T00:00:00Z\",\"stage\":\"build\",\"status\":\"running\",\"message\":\"Building\",\"progress\":20}\n\n";

        let stream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(chunk))]);

        let mut sse = parse_sse_stream(stream, "dep_eof".to_string());
        let ev1 = sse.next().await.unwrap().unwrap();
        assert_eq!(ev1.seq, 1);

        let err = sse.next().await.unwrap().unwrap_err();
        assert!(err.fix.contains("skoci watch dep_eof"));

        assert!(sse.next().await.is_none());
    }
    #[test]
    fn test_pin_normalization() {
        let valid_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let prefixed = format!("sha256:{valid_hex}");
        let upper = format!("SHA256:{}", valid_hex.to_ascii_uppercase());

        assert_eq!(
            skoci_api::models::normalize_spki_pin(&prefixed).unwrap(),
            prefixed
        );
        assert_eq!(
            skoci_api::models::normalize_spki_pin(valid_hex).unwrap(),
            prefixed
        );
        assert_eq!(
            skoci_api::models::normalize_spki_pin(&upper).unwrap(),
            prefixed
        );

        assert!(skoci_api::models::normalize_spki_pin("").is_none());
        assert!(skoci_api::models::normalize_spki_pin("   ").is_none());
        assert!(skoci_api::models::normalize_spki_pin("sha256:too_short").is_none());
        assert!(skoci_api::models::normalize_spki_pin(
            "not_hex_chars_0123456789abcdef0123456789abcdef0123456789abcdefgh"
        )
        .is_none());

        // Bad pin in client creation fails with bad_request
        let res = SkociClient::new("http://localhost:8787", None, Some("bad_pin".to_string()));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "bad_request");
    }

    #[test]
    fn test_https_ip_without_pin_requires_pin() {
        let res = SkociClient::new("https://127.0.0.1:8443", None, None);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, "pin_required");

        let res_v6 = SkociClient::new("https://[::1]:8443", None, None);
        assert!(res_v6.is_err());
        let err_v6 = res_v6.unwrap_err();
        assert_eq!(err_v6.code, "pin_required");
    }

    #[test]
    fn test_https_hostname_without_pin_allowed() {
        let res = SkociClient::new("https://api.skoci.example.com", None, None);
        assert!(res.is_ok());
    }

    #[test]
    fn test_http_ip_without_pin_allowed() {
        let res = SkociClient::new("http://127.0.0.1:8787", None, None);
        assert!(res.is_ok());
    }

    #[test]
    fn test_spki_computation_matches_der() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let der = cert.cert.der();

        let (_, parsed) = x509_parser::parse_x509_certificate(der).unwrap();
        let spki_der = parsed.tbs_certificate.subject_pki.raw;
        let pin = skoci_api::models::spki_pin_from_der(spki_der);

        assert!(pin.starts_with("sha256:"));
        assert_eq!(pin.len(), 71);

        let client = SkociClient::new("https://127.0.0.1:8443", None, Some(pin.clone())).unwrap();
        assert_eq!(client.pin(), Some(pin.as_str()));
    }
}
