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
        control_tls_bind: None,
        allow_magic_dns_production: false,
        require_approval_always: false,
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
        upload_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
        control_tls: Default::default(),
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
        control_tls_bind: None,
        allow_magic_dns_production: false,
        require_approval_always: false,
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

#[test]
fn test_smoke_classification_table_full() {
    use skoci_control::deploy::is_smoke_status_passed;

    let cases = [
        (Some(200), true),
        (Some(301), true),
        (Some(401), true),
        (Some(403), false),
        (Some(404), false),
        (Some(500), false),
        (None, false),
    ];
    for (status, expected_pass) in cases {
        let passed = status.map(is_smoke_status_passed).unwrap_or(false);
        assert_eq!(
            passed, expected_pass,
            "Failed classification for status {:?}",
            status
        );
    }
}

fn test_skoci_config(app: &str, domains: Vec<String>) -> skoci_api::models::SkociConfig {
    skoci_api::models::SkociConfig {
        version: 1,
        app: app.to_string(),
        build: Default::default(),
        run: skoci_api::models::RunConfig {
            port: 8080,
            health: "/".to_string(),
            health_timeout_secs: 10,
            memory: None,
            cpus: None,
        },
        domains,
        preview_domains: vec![],
        env: Default::default(),
        services: Default::default(),
    }
}

#[tokio::test]
async fn test_production_domain_policy_enforcement() {
    use skoci_control::deploy::check_production_domain_policy;

    let (state, _temp) = setup_test_state(10).await;

    // 1. Production with no domains and no base domain => Err(production_requires_domain)
    let cfg_empty = test_skoci_config("myapp", vec![]);
    let err = check_production_domain_policy(&state, "myapp", "production", &cfg_empty).unwrap_err();
    assert_eq!(err.code, "production_requires_domain");

    // 2. Production with magic-DNS domain only => Err(production_requires_domain)
    let cfg_magic = test_skoci_config("myapp", vec!["myapp.127.0.0.1.sslip.io".to_string()]);
    let err = check_production_domain_policy(&state, "myapp", "production", &cfg_magic).unwrap_err();
    assert_eq!(err.code, "production_requires_domain");

    // 3. Production with owned custom domain => Ok
    let cfg_owned = test_skoci_config("myapp", vec!["example.com".to_string()]);
    assert!(check_production_domain_policy(&state, "myapp", "production", &cfg_owned).is_ok());

    // 4. Preview with magic-DNS => Ok (preview allows magic DNS)
    assert!(check_production_domain_policy(&state, "myapp", "preview", &cfg_magic).is_ok());

    // 5. Operator override: allow_magic_dns_production = true => Ok for magic-DNS
    let mut config_override = (*state.config).clone();
    config_override.allow_magic_dns_production = true;
    let mut state_override = state.clone();
    state_override.config = Arc::new(config_override);
    assert!(check_production_domain_policy(&state_override, "myapp", "production", &cfg_magic).is_ok());
}

#[tokio::test]
async fn test_domain_check_api_endpoint() {
    use skoci_api::responses::DomainCheckResponse;

    let (mut state, _temp) = setup_test_state(10).await;
    let mut config = (*state.config).clone();
    config.public_ip = Some("203.0.113.10".parse().unwrap());
    state.config = Arc::new(config);

    let app = build_router(state);

    // 1. Host that does not resolve here and is not magic DNS
    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/check")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "unresolvable-test-host-99999.invalid" }).to_string()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let resp: ApiResponse<DomainCheckResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(resp.success);
    let data = resp.data.unwrap();
    assert!(!data.points_here);
    assert!(!data.magic_dns);
    assert_eq!(data.expected_ips, vec!["203.0.113.10"]);
    assert_eq!(data.record_hint, Some("A unresolvable-test-host-99999.invalid -> 203.0.113.10".to_string()));

    // 2. Magic DNS host (e.g. 1-2-3-4.sslip.io)
    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/check")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "1-2-3-4.sslip.io" }).to_string()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let resp: ApiResponse<DomainCheckResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(resp.success);
    let data = resp.data.unwrap();
    assert!(data.magic_dns);
    assert!(!data.points_here);
    // 3. Invalid host rejected with bad request (400)
    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/check")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "https://invalid-with-scheme.com" }).to_string()))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_estimated_deploy_duration_mean_calculation() {
    use chrono::{Duration as ChronoDuration, Utc};
    use skoci_control::db::DeploymentRecord;

    let db = Database::open_in_memory().unwrap();

    // 1. No history -> returns None
    let eta = db.get_estimated_deploy_duration("myapp", "production").unwrap();
    assert_eq!(eta, None);

    let now = Utc::now();

    // Insert 2 successful deployments (live) with durations 10s and 20s
    let dep1 = DeploymentRecord {
        id: "dep_1".to_string(),
        app: "myapp".to_string(),
        env: "production".to_string(),
        version: "v1".to_string(),
        kind: "deploy".to_string(),
        status: "live".to_string(),
        commit_hash: "abc".to_string(),
        image: "img".to_string(),
        container: "c1".to_string(),
        host_port: Some(8080),
        triggered_by: "human".to_string(),
        created_at: now - ChronoDuration::seconds(100),
        finished_at: Some(now - ChronoDuration::seconds(90)),
        error_json: None,
        evidence_json: Some(json!({ "duration_ms": 10000 }).to_string()),
    };
    db.insert_deployment(&dep1).unwrap();

    let dep2 = DeploymentRecord {
        id: "dep_2".to_string(),
        app: "myapp".to_string(),
        env: "production".to_string(),
        version: "v2".to_string(),
        kind: "deploy".to_string(),
        status: "degraded".to_string(),
        commit_hash: "def".to_string(),
        image: "img".to_string(),
        container: "c2".to_string(),
        host_port: Some(8081),
        triggered_by: "human".to_string(),
        created_at: now - ChronoDuration::seconds(80),
        finished_at: Some(now - ChronoDuration::seconds(60)),
        error_json: None,
        evidence_json: Some(json!({ "duration_ms": 20000 }).to_string()),
    };
    db.insert_deployment(&dep2).unwrap();

    // Mean of 10 and 20 is 15
    let eta = db.get_estimated_deploy_duration("myapp", "production").unwrap();
    assert_eq!(eta, Some(15));

    // Also check alias
    let eta_alias = db.mean_deployment_duration_secs("myapp", "production").unwrap();
    assert_eq!(eta_alias, Some(15));

    // Failed deployment should NOT affect calculation
    let dep_fail = DeploymentRecord {
        id: "dep_fail".to_string(),
        app: "myapp".to_string(),
        env: "production".to_string(),
        version: "v3".to_string(),
        kind: "deploy".to_string(),
        status: "failed".to_string(),
        commit_hash: "ghi".to_string(),
        image: "img".to_string(),
        container: "c3".to_string(),
        host_port: Some(8082),
        triggered_by: "human".to_string(),
        created_at: now - ChronoDuration::seconds(50),
        finished_at: Some(now - ChronoDuration::seconds(10)),
        error_json: None,
        evidence_json: None,
    };
    db.insert_deployment(&dep_fail).unwrap();

    let eta = db.get_estimated_deploy_duration("myapp", "production").unwrap();
    assert_eq!(eta, Some(15));

    // Different app/env should return None
    assert_eq!(db.get_estimated_deploy_duration("myapp", "preview").unwrap(), None);
    assert_eq!(db.get_estimated_deploy_duration("otherapp", "production").unwrap(), None);
}

#[tokio::test]
async fn test_compute_urls_hostname_precedence_d5() {
    let (mut state, _temp) = setup_test_state(10).await;
    state.db.ensure_app("myapp").unwrap();
    state.db.set_setting("default_app", "myapp").unwrap();

    let mut cfg = (*state.config).clone();
    cfg.public_ip = Some("198.51.100.1".parse().unwrap());
    cfg.tls = TlsMode::Acme;
    state.config = Arc::new(cfg);

    // 1. With hostname: only hostname URL, NO raw IP URL (D5 bug fixed)
    let hosts = vec!["myapp.example.com".to_string()];
    let urls = skoci_control::deploy::compute_urls(&state, "myapp", "production", &hosts);
    assert_eq!(urls, vec!["https://myapp.example.com".to_string()]);
    assert!(!urls.contains(&"http://198.51.100.1".to_string()));

    // 2. Without hostname: raw IP URL is advertised for default app
    let urls_empty = skoci_control::deploy::compute_urls(&state, "myapp", "production", &[]);
    assert_eq!(urls_empty, vec!["http://198.51.100.1".to_string()]);

    // 3. Without hostname, but for a non-default app: empty URLs
    state.db.ensure_app("otherapp").unwrap();
    let urls_other = skoci_control::deploy::compute_urls(&state, "otherapp", "production", &[]);
    assert!(urls_other.is_empty());

    // 4. Multiple hostnames: all hostnames advertised, NO raw IP URL
    let multi_hosts = vec!["app1.example.com".to_string(), "app2.example.com".to_string()];
    let urls_multi = skoci_control::deploy::compute_urls(&state, "myapp", "production", &multi_hosts);
    assert_eq!(
        urls_multi,
        vec![
            "https://app1.example.com".to_string(),
            "https://app2.example.com".to_string()
        ]
    );
    assert!(!urls_multi.contains(&"http://198.51.100.1".to_string()));
}

#[test]
fn test_resolve_project_config_prefers_dot_skoci_d1() {
    let temp = TestDir::new();
    let root = temp.path();
    std::fs::create_dir_all(root.join(".skoci")).unwrap();
    std::fs::write(root.join(".skoci/skoci.yaml"), "version: 1\napp: my-app\n").unwrap();
    std::fs::write(root.join("skoci.yaml"), "version: 1\napp: legacy\n").unwrap();

    let resolved = skoci_control::deploy::resolve_project_config(root).unwrap();
    assert_eq!(resolved, root.join(".skoci/skoci.yaml"));
}

#[test]
fn test_resolve_project_config_rejects_legacy_root_config() {
    let temp = TestDir::new();
    let root = temp.path();
    std::fs::write(root.join("skoci.yaml"), "version: 1\napp: legacy\n").unwrap();

    let err = skoci_control::deploy::resolve_project_config(root).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.fix.contains("skoci init"), "{}", err.fix);
}

#[test]
fn test_resolve_project_config_missing_names_both_locations() {
    let temp = TestDir::new();
    std::fs::write(temp.path().join("index.html"), "hello").unwrap();

    let err = skoci_control::deploy::resolve_project_config(temp.path()).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.message.contains(".skoci/skoci.yaml"), "{}", err.message);
    assert!(err.fix.contains(".skoci/skoci.yaml"), "{}", err.fix);
}

#[test]
fn test_resolve_project_config_rejects_symlinked_config() {
    let temp = TestDir::new();
    let root = temp.path();
    std::fs::create_dir_all(root.join(".skoci")).unwrap();
    std::fs::write(root.join("outside.yaml"), "version: 1\napp: my-app\n").unwrap();
    std::os::unix::fs::symlink(root.join("outside.yaml"), root.join(".skoci/skoci.yaml")).unwrap();

    let err = skoci_control::deploy::resolve_project_config(root).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.message.contains("symlink"), "{}", err.message);
}

#[tokio::test]
async fn test_base_domain_accessor_prefers_db_over_startup_config() {
    let (mut state, _temp) = setup_test_state(10).await;
    let mut config = (*state.config).clone();
    config.base_domain = Some("startup.example.com".to_string());
    config.api_domain = Some("skoci.startup.example.com".to_string());
    state.config = Arc::new(config);

    assert_eq!(state.base_domain(), Some("startup.example.com".to_string()));
    assert_eq!(state.api_domain(), Some("skoci.startup.example.com".to_string()));

    state.db.set_setting("base_domain", "override.example.com").unwrap();

    assert_eq!(state.base_domain(), Some("override.example.com".to_string()));
    assert_eq!(state.api_domain(), Some("skoci.override.example.com".to_string()));
}

#[tokio::test]
async fn test_domain_set_primary_rejects_magic_dns_and_raw_ip() {
    let (state, _temp) = setup_test_state(10).await;
    let app = build_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/primary")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "192.168.1.100", "force": true }).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/primary")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "::1", "force": true }).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/primary")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "192-168-1-100.sslip.io", "force": true }).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_domain_set_primary_success_with_force() {
    use chrono::Utc;
    use skoci_api::responses::DomainSetPrimaryResponse;
    use skoci_control::db::RouteRecord;

    let (state, _temp) = setup_test_state(10).await;

    let route_rec = RouteRecord {
        app: "my-app".to_string(),
        env: "production".to_string(),
        container: "c_my_app_prod".to_string(),
        host_port: 3000,
        version: "v1".to_string(),
        hosts: vec!["my-app.old.example.com".to_string()],
        deployed_at: Utc::now(),
    };
    state.db.upsert_route(&route_rec).unwrap();

    let app = build_router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/primary")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {MASTER_TOKEN}"))
        .body(Body::from(json!({ "host": "newdomain.com", "force": true }).to_string()))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let resp: ApiResponse<DomainSetPrimaryResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(resp.success);
    let data = resp.data.unwrap();

    assert_eq!(data.base_domain, "newdomain.com");
    assert_eq!(data.api_domain, "skoci.newdomain.com");
    assert_eq!(data.previous_base_domain, None);
    assert!(data.dns_check_skipped);
    assert_eq!(data.updated_routes, vec!["my-app:production".to_string()]);
    assert!(data.certificate_hosts.contains(&"skoci.newdomain.com".to_string()));
    assert!(data.certificate_hosts.contains(&"my-app.newdomain.com".to_string()));

    assert_eq!(
        state.db.get_setting("base_domain").unwrap(),
        Some("newdomain.com".to_string())
    );
    assert_eq!(state.base_domain(), Some("newdomain.com".to_string()));
    assert_eq!(state.api_domain(), Some("skoci.newdomain.com".to_string()));

    let routes = state.db.list_routes().unwrap();
    let updated = routes
        .iter()
        .find(|r| r.app == "my-app" && r.env == "production")
        .unwrap();
    assert!(updated.hosts.contains(&"my-app.newdomain.com".to_string()));
}

#[tokio::test]
async fn test_domain_set_primary_requires_admin() {
    let (state, _temp) = setup_test_state(10).await;
    let (agent_token, hash, _) = generate_agent_token("some-app");
    state.db.insert_agent_token(&hash, "some-app", "agent-1").unwrap();
    let app = build_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains/primary")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {agent_token}"))
        .body(Body::from(json!({ "host": "newdomain.com", "force": true }).to_string()))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

fn agent_subject(app: &str) -> skoci_control::auth::Subject {
    skoci_control::auth::Subject::Agent {
        app: app.to_string(),
        name: "agent-1".to_string(),
        id: "abc123def456".to_string(),
    }
}

fn production_route(app: &str) -> skoci_control::db::RouteRecord {
    skoci_control::db::RouteRecord {
        app: app.to_string(),
        env: "production".to_string(),
        container: format!("skoci-{app}-production-v1"),
        host_port: 3000,
        version: "v1".to_string(),
        hosts: vec![format!("{app}.example.com")],
        deployed_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn test_first_promotion_by_agent_needs_no_approval() {
    use skoci_control::api::promote::promotion_requires_approval;

    let (state, _temp) = setup_test_state(10).await;
    state.db.ensure_app("fresh-app").unwrap();

    let needs = promotion_requires_approval(&state, &agent_subject("fresh-app"), "fresh-app").unwrap();
    assert!(!needs);
    assert!(state.db.list_pending_approvals(Some("fresh-app")).unwrap().is_empty());
}

#[tokio::test]
async fn test_promotion_by_agent_with_live_production_needs_approval() {
    use skoci_control::api::promote::promotion_requires_approval;

    let (state, _temp) = setup_test_state(10).await;
    state.db.ensure_app("live-app").unwrap();
    state.db.upsert_route(&production_route("live-app")).unwrap();

    assert!(promotion_requires_approval(&state, &agent_subject("live-app"), "live-app").unwrap());
    assert!(!promotion_requires_approval(&state, &skoci_control::auth::Subject::Admin, "live-app").unwrap());
}

#[tokio::test]
async fn test_require_approval_always_restores_approval_for_first_promotion() {
    use skoci_control::api::promote::promotion_requires_approval;

    let (mut state, _temp) = setup_test_state(10).await;
    let mut config = (*state.config).clone();
    config.require_approval_always = true;
    state.config = Arc::new(config);
    state.db.ensure_app("fresh-app").unwrap();

    assert!(promotion_requires_approval(&state, &agent_subject("fresh-app"), "fresh-app").unwrap());
    assert!(!promotion_requires_approval(&state, &skoci_control::auth::Subject::Admin, "fresh-app").unwrap());
}

struct ProbeServer {
    port: u16,
    serving: Arc<std::sync::atomic::AtomicBool>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
}

async fn spawn_probe_server(serving_from_start: bool) -> ProbeServer {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = Arc::new(AtomicBool::new(serving_from_start));
    let connections = Arc::new(AtomicUsize::new(0));

    let serving_task = serving.clone();
    let connections_task = connections.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            connections_task.fetch_add(1, Ordering::SeqCst);
            let ready = serving_task.load(Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let status = if ready { "200 OK" } else { "503 Service Unavailable" };
                let resp = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });

    ProbeServer { port, serving, connections }
}

fn state_probing_port(state: &mut AppState, port: u16) {
    let mut config = (*state.config).clone();
    config.http_bind = format!("127.0.0.1:{port}").parse().unwrap();
    state.config = Arc::new(config);
}

#[tokio::test]
async fn test_wait_for_public_url_serving_url_costs_one_probe() {
    use skoci_control::deploy::wait_for_public_url_with_budget;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let (mut state, _temp) = setup_test_state(10).await;
    let server = spawn_probe_server(true).await;
    state_probing_port(&mut state, server.port);

    let started = Instant::now();
    let ready = wait_for_public_url_with_budget(
        &state,
        "http://myapp.example.com",
        Duration::from_secs(10),
        Duration::from_secs(60),
        Duration::from_secs(1),
        |_| {},
    )
    .await;

    assert!(ready);
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn test_wait_for_public_url_waits_until_url_starts_serving() {
    use skoci_control::deploy::wait_for_public_url_with_budget;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    let (mut state, _temp) = setup_test_state(10).await;
    let server = spawn_probe_server(false).await;
    state_probing_port(&mut state, server.port);

    let serving = server.serving.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        serving.store(true, Ordering::SeqCst);
    });

    let ready = wait_for_public_url_with_budget(
        &state,
        "http://myapp.example.com",
        Duration::from_secs(10),
        Duration::from_secs(60),
        Duration::from_millis(50),
        |_| {},
    )
    .await;

    assert!(ready);
    assert!(server.connections.load(Ordering::SeqCst) > 1);
}

#[tokio::test]
async fn test_wait_for_public_url_gives_up_at_deadline() {
    use skoci_control::deploy::wait_for_public_url_with_budget;
    use std::time::{Duration, Instant};

    let (mut state, _temp) = setup_test_state(10).await;
    let server = spawn_probe_server(false).await;
    state_probing_port(&mut state, server.port);

    let started = Instant::now();
    let ready = wait_for_public_url_with_budget(
        &state,
        "http://myapp.example.com",
        Duration::from_millis(300),
        Duration::from_secs(60),
        Duration::from_millis(50),
        |_| {},
    )
    .await;

    assert!(!ready);
    assert!(started.elapsed() < Duration::from_secs(5));
}
