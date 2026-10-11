use tokio_util::bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{
    ApproveRequest, DbBackupRequest, DbRestoreRequest, DefaultAppRequest, DeployRequest,
    DeploymentsQuery, LogsQuery, PromoteRequest, RollbackRequest, SecretSetRequest,
    TokenCreateRequest,
};
use skoci_api::responses::{
    AppDeleteResponse, AppSummary, ApprovalInfo, ApproveResponse, AuditEntry,
    DbBackupListResponse, DbBackupResponse, DbRestoreResponse, DefaultAppResponse,
    DeployAccepted, DeployEvent, DeployStage, DeploymentInfo, DoctorResponse,
    LogsResponse, PromoteResponse, SecretListResponse, StatusResponse,
    TokenCreateResponse, TokenListResponse, TokenRevokeResponse,
};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct SkociClient {
    base_url: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl SkociClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
            client,
        }
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
            SkociError::internal(
                format!("Gagal menghubungi server Skoci: {e}"),
                "Periksa apakah server Skoci berjalan dan URL server sudah benar (SKOCI_SERVER_URL)",
            )
        })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            SkociError::internal(
                format!("Gagal membaca respons dari server Skoci: {e}"),
                "Periksa koneksi jaringan dan server Skoci",
            )
        })?;

        match serde_json::from_str::<ApiResponse<T>>(&text) {
            Ok(parsed) => Ok(parsed),
            Err(_) => {
                let snippet: String = text.chars().take(200).collect();
                Err(SkociError::internal(
                    format!(
                        "Respons bukan format JSON yang valid (HTTP {}): {}",
                        status.as_u16(),
                        snippet
                    ),
                    "Periksa apakah SKOCI_SERVER_URL mengarah ke server Skoci yang benar dan bukan layanan lain",
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
                format!("Gagal membuka file context {}: {e}", path.display()),
                "Pastikan file arsip context tersedia dan dapat dibaca",
            )
        })?;
        let total_bytes = file.metadata().await.map_err(|e| {
            SkociError::internal(format!("Gagal membaca metadata file: {e}"), "Coba deploy kembali")
        })?.len();

        let sent = Arc::new(AtomicU64::new(0));
        let progress = Arc::new(progress);

        let stream = tokio_util::io::ReaderStream::new(file).map(move |res| {
            if let Ok(bytes) = &res {
                let cur = sent.fetch_add(bytes.len() as u64, Ordering::SeqCst)
                    + bytes.len() as u64;
                progress(cur, total_bytes);
            }
            res
        });

        let body = reqwest::Body::wrap_stream(stream);
        let mut url = format!("{}/api/deploy?app={}&env={}", self.base_url, req.app, req.env);
        if let Some(commit) = &req.commit {
            url.push_str(&format!("&commit={}", commit));
        }
        let rb = self.client.post(&url)
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

        let init = async move {
            let resp = rb.send().await.map_err(|e| {
                SkociError::internal(
                    format!("Gagal menghubungkan ke event stream: {e}"),
                    format!("Gunakan `skoci watch {id_str}` untuk mencoba kembali"),
                )
            })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(SkociError::internal(
                    format!("Server merespons error HTTP {status}: {text}"),
                    format!("Gunakan `skoci watch {id_str}` untuk mencoba kembali"),
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

    pub async fn rollback(
        &self,
        req: RollbackRequest,
    ) -> SkociResult<ApiResponse<DeployAccepted>> {
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

    Box::pin(futures_util::stream::unfold(state, |mut state| Box::pin(async move {
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
                                        format!("Gagal mem-parsing SSE event: {e}"),
                                        "Periksa format data SSE dari server",
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
                            "Koneksi event terputus sebelum deployment selesai",
                            format!("Gunakan `skoci watch {}` untuk menghubungkan kembali", state.deployment_id),
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
                            format!("Koneksi SSE terputus dengan error: {e}"),
                            format!("Gunakan `skoci watch {}` untuk menghubungkan kembali", state.deployment_id),
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
                                "Koneksi event terputus sebelum deployment selesai",
                                format!("Gunakan `skoci watch {}` untuk menghubungkan kembali", state.deployment_id),
                            )),
                            state,
                        ));
                    }
                    return None;
                }
            }
        }
    })))
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
}
