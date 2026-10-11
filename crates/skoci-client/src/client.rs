use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::ApiResponse;
use skoci_api::requests::{
    ApproveRequest, DbBackupRequest, DbRestoreRequest, DeployRequest, PromoteRequest,
    RollbackRequest, SecretSetRequest, TokenCreateRequest,
};
use skoci_api::responses::{
    ApproveResponse, DbBackupResponse, DbRestoreResponse, DeployResponse, DoctorResponse,
    LogsResponse, PromoteResponse, RollbackResponse, SecretListResponse, StatusResponse,
    TokenCreateResponse, TokenListResponse,
};

#[derive(Clone)]
pub struct SkociClient {
    base_url: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl SkociClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
            client: reqwest::Client::new(),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(token) = &self.token {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    pub async fn deploy(&self, req: DeployRequest) -> SkociResult<ApiResponse<DeployResponse>> {
        let url = format!("{}/api/deploy", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<DeployResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn status(&self, app: &str) -> SkociResult<ApiResponse<StatusResponse>> {
        let url = format!("{}/api/status/{app}", self.base_url);
        self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<StatusResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn logs(&self, app: &str, tail: usize) -> SkociResult<ApiResponse<LogsResponse>> {
        let url = format!("{}/api/logs/{app}?tail={tail}", self.base_url);
        self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<LogsResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn secret_set(
        &self,
        app: &str,
        key: &str,
        value: &str,
    ) -> SkociResult<ApiResponse<()>> {
        let url = format!("{}/api/secret", self.base_url);
        let req = SecretSetRequest {
            app: app.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        };
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<()>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn secret_list(&self, app: &str) -> SkociResult<ApiResponse<SecretListResponse>> {
        let url = format!("{}/api/secret/{app}", self.base_url);
        self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<SecretListResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn promote(&self, req: PromoteRequest) -> SkociResult<ApiResponse<PromoteResponse>> {
        let url = format!("{}/api/promote", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<PromoteResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn approve(&self, req: ApproveRequest) -> SkociResult<ApiResponse<ApproveResponse>> {
        let url = format!("{}/api/approve", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<ApproveResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn rollback(
        &self,
        req: RollbackRequest,
    ) -> SkociResult<ApiResponse<RollbackResponse>> {
        let url = format!("{}/api/rollback", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<RollbackResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn doctor(&self) -> SkociResult<ApiResponse<DoctorResponse>> {
        let url = format!("{}/api/doctor", self.base_url);
        self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<DoctorResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn db_backup(
        &self,
        req: DbBackupRequest,
    ) -> SkociResult<ApiResponse<DbBackupResponse>> {
        let url = format!("{}/api/db/backup", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<DbBackupResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn db_restore(
        &self,
        req: DbRestoreRequest,
    ) -> SkociResult<ApiResponse<DbRestoreResponse>> {
        let url = format!("{}/api/db/restore", self.base_url);
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<DbRestoreResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn token_create(
        &self,
        app: &str,
        name: &str,
    ) -> SkociResult<ApiResponse<TokenCreateResponse>> {
        let url = format!("{}/api/tokens", self.base_url);
        let req = TokenCreateRequest {
            app: app.to_string(),
            name: name.to_string(),
        };
        self.client
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<TokenCreateResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }

    pub async fn token_list(&self, app: &str) -> SkociResult<ApiResponse<TokenListResponse>> {
        let url = format!("{}/api/tokens/{app}", self.base_url);
        self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Cek koneksi ke server Skoci"))?
            .json::<ApiResponse<TokenListResponse>>()
            .await
            .map_err(|e| SkociError::bad_request(e.to_string(), "Gagal membaca respons server"))
    }
}
