use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use skoci_api::models::ApiResponse;
use skoci_api::responses::StatusResponse;
use skoci_control::api::{create_router, AppState};
use skoci_control::approval::ApprovalService;
use skoci_control::auth::AuthManager;
use skoci_control::db::Database;
use parking_lot::Mutex;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretStore;
use skoci_proxy::router::ProxyRouter;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_control_api_status_and_auth() {
    let db = Database::open_in_memory().unwrap();
    let auth = Arc::new(AuthManager::new("master_test_token"));
    let approvals = ApprovalService::new(db.clone());
    db.upsert_app("my-toko", "default", Some("v1"), &[format!("my-toko.example.com")]).unwrap();

    let state = AppState {
        db: db.clone(),
        auth,
        approvals,
        proxy: ProxyRouter::new(),
        docker: Arc::new(DockerClient::new()),
        secrets: Arc::new(Mutex::new(SecretStore::new("test_seed"))),
        backup: Arc::new(BackupManager::new(std::env::temp_dir())),
    };

    let router = create_router(state);

    // Call status endpoint
    let req = Request::builder()
        .uri("/api/status/my-toko")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let api_res: ApiResponse<StatusResponse> = serde_json::from_slice(&bytes).unwrap();
    assert!(api_res.success);
    assert_eq!(api_res.data.unwrap().app, "my-toko");
}

#[test]
fn test_db_approval_workflow() {
    let db = Database::open_in_memory().unwrap();
    let approvals = ApprovalService::new(db.clone());

    let promote_res = approvals
        .request_approval("toko", "promote", "production", "Ready for launch", "agent_1")
        .unwrap();
    assert_eq!(promote_res.status, "pending_approval");

    let status = db.get_approval_status(&promote_res.request_id).unwrap();
    assert_eq!(status.as_deref(), Some("pending"));

    let approved = approvals.approve(&promote_res.request_id, "admin").unwrap();
    assert!(approved);

    let status_after = db.get_approval_status(&promote_res.request_id).unwrap();
    assert_eq!(status_after.as_deref(), Some("approved"));
}

#[tokio::test]
async fn test_scoped_token_isolation() {
    let db = Database::open_in_memory().unwrap();
    let auth = Arc::new(AuthManager::new("master_test_token"));
    let approvals = ApprovalService::new(db.clone());

    let state = AppState {
        db: db.clone(),
        auth,
        approvals,
        proxy: ProxyRouter::new(),
        docker: Arc::new(DockerClient::new()),
        secrets: Arc::new(Mutex::new(SecretStore::new("test_seed"))),
        backup: Arc::new(BackupManager::new(std::env::temp_dir())),
    };

    let router = create_router(state);

    // 1. Deploy toko using toko token -> should succeed
    let deploy_toko = serde_json::json!({
        "app": "toko",
        "env": "preview"
    });
    let req1 = Request::builder()
        .method("POST")
        .uri("/api/deploy")
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer sk_agent_toko_12345678")
        .body(Body::from(serde_json::to_vec(&deploy_toko).unwrap()))
        .unwrap();

    let res1 = router.clone().oneshot(req1).await.unwrap();
    let bytes1 = res1.into_body().collect().await.unwrap().to_bytes();
    let api_res1: ApiResponse<serde_json::Value> = serde_json::from_slice(&bytes1).unwrap();
    assert!(api_res1.success);

    // 2. Deploy blog using toko token -> MUST fail with scope_violation
    let deploy_blog = serde_json::json!({
        "app": "blog",
        "env": "preview"
    });
    let req2 = Request::builder()
        .method("POST")
        .uri("/api/deploy")
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer sk_agent_toko_12345678")
        .body(Body::from(serde_json::to_vec(&deploy_blog).unwrap()))
        .unwrap();

    let res2 = router.clone().oneshot(req2).await.unwrap();
    let bytes2 = res2.into_body().collect().await.unwrap().to_bytes();
    let api_res2: ApiResponse<serde_json::Value> = serde_json::from_slice(&bytes2).unwrap();
    assert!(!api_res2.success);
    assert_eq!(api_res2.error.unwrap().code, "scope_violation");
}

#[tokio::test]
async fn test_reject_and_latest_deployment() {
    let db = Database::open_in_memory().unwrap();
    let auth = Arc::new(AuthManager::new("master_test_token"));
    let approvals = ApprovalService::new(db.clone());

    let state = AppState {
        db: db.clone(),
        auth,
        approvals: approvals.clone(),
        proxy: ProxyRouter::new(),
        docker: Arc::new(DockerClient::new()),
        secrets: Arc::new(Mutex::new(SecretStore::new("test_seed"))),
        backup: Arc::new(BackupManager::new(std::env::temp_dir())),
    };

    let router = create_router(state);

    // 1. Create a pending approval
    let prom = approvals.request_approval("toko", "promote", "prod", "Test reason", "agent_1").unwrap();

    // 2. Reject it via /api/reject with master human token
    let reject_payload = serde_json::json!({
        "request_id": prom.request_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/reject")
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer master_test_token")
        .body(Body::from(serde_json::to_vec(&reject_payload).unwrap()))
        .unwrap();

    let res = router.clone().oneshot(req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let api_res: ApiResponse<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert!(api_res.success);
    assert_eq!(api_res.data.unwrap()["action"], "rejected");

    // 3. Test /api/deployments/latest
    let req2 = Request::builder()
        .method("GET")
        .uri("/api/deployments/latest")
        .body(Body::empty())
        .unwrap();

    let res2 = router.oneshot(req2).await.unwrap();
    let bytes2 = res2.into_body().collect().await.unwrap().to_bytes();
    let api_res2: ApiResponse<serde_json::Value> = serde_json::from_slice(&bytes2).unwrap();
    assert!(api_res2.success);
}
