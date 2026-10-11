use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use skoci_api::models::ApiResponse;
use skoci_api::responses::DbRestoreResponse;
use skoci_control::api::build_router;
use skoci_control::auth::generate_agent_token;
use skoci_control::config::{ServerConfig, TlsMode};
use skoci_control::db::Database;
use skoci_control::events::JobRegistry;
use skoci_control::ratelimit::AuthRateLimiter;
use skoci_control::session::SessionStore;
use skoci_control::state::AppState;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretCipher;
use skoci_node::services::ServiceManager;
use skoci_proxy::ProxyRouter;
use std::sync::Arc;
use tower::ServiceExt;

const MASTER_TOKEN: &str = "sk_master_secret_token_1234567890_abcdefghij";

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("skoci_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&p);
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn setup_test_state(max_upload_mb: u64) -> (AppState, TestDir) {
    let temp_dir = TestDir::new();
    let data_dir = temp_dir.path().to_path_buf();
    let db = Database::open_in_memory().expect("db");

    let config = Arc::new(ServerConfig {
        bind: "127.0.0.1:8787".parse().unwrap(),
        data_dir: data_dir.clone(),
        master_token: MASTER_TOKEN.to_string(),
        http_bind: "127.0.0.1:80".parse().unwrap(),
        https_bind: "127.0.0.1:443".parse().unwrap(),
        base_domain: None,
        api_domain: None,
        tls: TlsMode::Off,
        acme_email: None,
        acme_staging: false,
        max_upload_mb,
        public_ip: None,
    });

    let secret_cipher = Arc::new(SecretCipher::from_key([42u8; 32]));
    let docker = DockerClient::new();
    let service_manager = ServiceManager::new(docker.clone());
    let backup_manager = Arc::new(BackupManager::new(&data_dir, docker.clone()));
    let router = ProxyRouter::new();
    let sessions = SessionStore::new();
    let rate_limiter = AuthRateLimiter::new();
    let jobs = JobRegistry::new();

    let state = AppState {
        config,
        db,
        sessions,
        rate_limiter,
        jobs,
        docker,
        router,
        secret_cipher,
        service_manager,
        backup_manager,
        proxy_status: None,
    };

    (state, temp_dir)
}

#[tokio::test]
async fn test_unauthenticated_requests_return_401() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);

    // 1. GET /api/apps
    let req = Request::builder()
        .uri("/api/apps")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. GET /api/audits
    let req = Request::builder()
        .uri("/api/audits")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 3. POST /api/db/restore
    let req = Request::builder()
        .uri("/api/db/restore")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "test",
                "service": "db",
                "backup_id": "b1"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_forged_agent_token_returns_401() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/apps")
        .method("GET")
        .header("authorization", "Bearer sk_agent_x_deadbeef")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_scoped_token_isolation() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    // Create token for "app-a"
    let (token, hash, _) = generate_agent_token("app-a");
    state.db.insert_agent_token(&hash, "app-a", "agent-a").unwrap();
    state.db.ensure_app("app-a").unwrap();
    state.db.ensure_app("app-b").unwrap();

    // Access app-a secrets: allowed
    let req = Request::builder()
        .uri("/api/secret/app-a")
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Access app-b secrets: 403 scope_violation
    let req = Request::builder()
        .uri("/api/secret/app-b")
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_human_token_rejected_returns_401() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/apps")
        .method("GET")
        .header("authorization", "Bearer sk_human_anything")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_revoked_token_returns_401() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    let (token, hash, _) = generate_agent_token("my-app");
    state.db.insert_agent_token(&hash, "my-app", "agent-1").unwrap();
    state.db.ensure_app("my-app").unwrap();

    // Works before revocation
    let req = Request::builder()
        .uri("/api/secret/my-app")
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Revoke token
    let revoked = state.db.revoke_agent_token("my-app", "agent-1").unwrap();
    assert_eq!(revoked, 1);

    // Fails after revocation
    let req = Request::builder()
        .uri("/api/secret/my-app")
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_audit_list_never_contains_raw_token() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    let (token, hash, _) = generate_agent_token("secret-app");
    state.db.insert_agent_token(&hash, "secret-app", "agent-sec").unwrap();
    state.db.ensure_app("secret-app").unwrap();

    // Perform an action with the agent token
    let req = Request::builder()
        .uri("/api/secret")
        .method("POST")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "secret-app",
                "key": "DATABASE_URL",
                "value": "postgres://localhost"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Query audits with master token
    let req = Request::builder()
        .uri("/api/audits")
        .method("GET")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Raw token must NEVER appear in audit responses
    assert!(!body_str.contains(&token), "Audit log must not contain raw token!");
    assert!(body_str.contains("agent:secret-app/agent-sec#"), "Audit must record redacted actor!");
}

#[tokio::test]
async fn test_agent_production_deploy_requires_approval() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    let (token, hash, _) = generate_agent_token("prod-app");
    state.db.insert_agent_token(&hash, "prod-app", "agent-p").unwrap();
    state.db.ensure_app("prod-app").unwrap();

    let req = Request::builder()
        .uri("/api/deploy?app=prod-app&env=production")
        .method("POST")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(vec![0u8; 100]))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN); // approval_required maps to 403

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("approval_required") || body_str.contains("promote"));
}

#[tokio::test]
async fn test_agent_db_restore_creates_approval_not_executed() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    let (token, hash, _) = generate_agent_token("restore-app");
    state.db.insert_agent_token(&hash, "restore-app", "agent-r").unwrap();
    state.db.ensure_app("restore-app").unwrap();

    let req = Request::builder()
        .uri("/api/db/restore")
        .method("POST")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "restore-app",
                "service": "db",
                "backup_id": "db_20261011_000000"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let parsed: ApiResponse<DbRestoreResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(parsed.success);

    let data = parsed.data.unwrap();
    assert_eq!(data.status, "pending_approval");
    assert!(data.request_id.is_some());

    // Verify approval exists in DB with status pending
    let appr = state
        .db
        .get_approval(data.request_id.as_ref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(appr.status, "pending");
}

#[tokio::test]
async fn test_cookie_session_csrf_protection() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());

    let (session_id, _) = state.sessions.create_session();
    state.db.ensure_app("csrf-app").unwrap();

    // 1. POST with cookie session but WITHOUT X-Skoci-Dashboard header -> 403 Forbidden
    let req = Request::builder()
        .uri("/api/secret")
        .method("POST")
        .header("cookie", format!("skoci_session={session_id}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "csrf-app",
                "key": "CSRF_KEY",
                "value": "secret"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. POST with cookie session AND X-Skoci-Dashboard: 1 header -> 200 OK
    let req = Request::builder()
        .uri("/api/secret")
        .method("POST")
        .header("cookie", format!("skoci_session={session_id}"))
        .header("x-skoci-dashboard", "1")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "csrf-app",
                "key": "CSRF_KEY",
                "value": "secret"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_rate_limit_after_10_bad_tokens() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);

    // Send 10 bad tokens
    for _ in 0..10 {
        let req = Request::builder()
            .uri("/api/apps")
            .method("GET")
            .header("authorization", "Bearer bad_token_123")
            .header("x-forwarded-for", "192.168.1.100")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    // 11th request from same IP -> 429 Too Many Requests
    let req = Request::builder()
        .uri("/api/apps")
        .method("GET")
        .header("authorization", "Bearer bad_token_123")
        .header("x-forwarded-for", "192.168.1.100")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_secret_value_with_newline_returns_400() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state.clone());
    state.db.ensure_app("app-nl").unwrap();

    let req = Request::builder()
        .uri("/api/secret")
        .method("POST")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "app": "app-nl",
                "key": "MULTI_LINE",
                "value": "hello\nworld"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("newline"));
}

#[tokio::test]
async fn test_upload_over_limit_returns_413() {
    // 1 MB limit
    let (state, _temp) = setup_test_state(1).await;
    let app = build_router(state.clone());
    state.db.ensure_app("upload-app").unwrap();

    // 1.5 MB dummy body
    let large_body = vec![0u8; 1_500_000];

    let req = Request::builder()
        .uri("/api/deploy?app=upload-app&env=preview")
        .method("POST")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(large_body))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn test_master_token_min_length_validation() {
    let temp_dir = TestDir::new();
    let cfg = ServerConfig {
        bind: "127.0.0.1:8787".parse().unwrap(),
        data_dir: temp_dir.path().to_path_buf(),
        master_token: "too_short".to_string(), // < 32 chars
        http_bind: "127.0.0.1:80".parse().unwrap(),
        https_bind: "127.0.0.1:443".parse().unwrap(),
        base_domain: None,
        api_domain: None,
        tls: TlsMode::Off,
        acme_email: None,
        acme_staging: false,
        max_upload_mb: 10,
        public_ip: None,
    };

    let res = skoci_control::server::run_server(cfg).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("32 characters"));
}

#[test]
fn test_legacy_v01_database_is_migrated_and_old_tokens_revoked() {
    let dir = TestDir::new();
    let path = dir.path().join("skoci.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE apps (name TEXT PRIMARY KEY, server TEXT NOT NULL, live_version TEXT, domains TEXT, updated_at TEXT NOT NULL);
            CREATE TABLE deployments (id TEXT PRIMARY KEY, app TEXT NOT NULL, version TEXT NOT NULL, commit_hash TEXT NOT NULL, env TEXT NOT NULL, status TEXT NOT NULL, evidence TEXT NOT NULL, created_at TEXT NOT NULL);
            CREATE TABLE approvals (id TEXT PRIMARY KEY, app TEXT NOT NULL, action TEXT NOT NULL, target_env TEXT NOT NULL, reason TEXT NOT NULL, status TEXT NOT NULL, requested_by TEXT NOT NULL, approved_by TEXT, created_at TEXT NOT NULL);
            CREATE TABLE audit_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, actor TEXT NOT NULL, actor_type TEXT NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, details TEXT NOT NULL);
            CREATE TABLE secrets (app TEXT NOT NULL, key TEXT NOT NULL, ciphertext_hex TEXT NOT NULL, nonce_hex TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (app, key));
            CREATE TABLE agent_tokens (token_hash TEXT PRIMARY KEY, app TEXT NOT NULL, name TEXT NOT NULL, created_at TEXT NOT NULL);
            INSERT INTO apps VALUES ('belajarpaper', 'default', 'v_1', '[]', '2026-10-11T02:02:57Z');
            INSERT INTO deployments VALUES ('dep_1', 'belajarpaper', 'v_1', 'HEAD', 'preview', 'live', '{}', '2026-10-11T02:02:57Z');
            INSERT INTO audit_logs (timestamp, actor, actor_type, action, target, details) VALUES ('2026-10-11T02:02:57Z', 'sk_agent_belajarpaper_ab43', 'agent', 'deploy', 'belajarpaper', 'x');
            INSERT INTO agent_tokens VALUES ('leakedhash', 'belajarpaper', 'laptop-agent', '2026-10-11T02:00:00Z');
            "#,
        )
        .unwrap();
    }

    let db = Database::open(&path).expect("legacy database must open");
    assert!(db.get_agent_token_by_hash("leakedhash").unwrap().is_none());
    assert!(db.list_deployments(None, 10).unwrap().is_empty());
    assert!(db.list_audits(10).unwrap().iter().all(|a| !a.actor.starts_with("sk_")));

    db.ensure_app("belajarpaper").unwrap();
    assert_eq!(db.list_apps().unwrap().len(), 1);
    db.insert_agent_token("newhash", "belajarpaper", "laptop").unwrap();
    assert!(db.get_agent_token_by_hash("newhash").unwrap().is_some());
}

#[tokio::test]
async fn test_dashboard_login_is_rate_limited_per_forwarded_client() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);
    let login = |ip: &'static str, token: &'static str| {
        Request::builder()
            .method("POST")
            .uri("/api/session")
            .header("content-type", "application/json")
            .header("x-forwarded-for", ip)
            .body(Body::from(json!({ "token": token }).to_string()))
            .unwrap()
    };

    for _ in 0..10 {
        let res = app.clone().oneshot(login("203.0.113.7", "wrong")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
    let res = app.clone().oneshot(login("203.0.113.7", MASTER_TOKEN)).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    let res = app.clone().oneshot(login("198.51.100.9", MASTER_TOKEN)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_logout_requires_authentication() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/session")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
