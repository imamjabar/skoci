use skoci_api::error::SkociError;
use skoci_api::models::{ActionPermission, ApiResponse, SkociConfig, SkociPolicy};

#[test]
fn test_error_serialization() {
    let err = SkociError::secret_missing("DATABASE_URL");
    let json = serde_json::to_string(&err).unwrap();
    assert!(json.contains("\"code\":\"secret_missing\""));
    assert!(json.contains("Secret 'DATABASE_URL' belum diset"));

    let deserialized: SkociError = serde_json::from_str(&json).unwrap();
    assert_eq!(err, deserialized);
}

#[test]
fn test_api_response_wrapper() {
    let res = ApiResponse::ok("healthy", "System healthy");
    assert!(res.success);
    assert_eq!(res.summary, "System healthy");

    let res_err = ApiResponse::<()>::err(SkociError::unauthorized("Token kedaluwarsa"));
    assert!(!res_err.success);
    assert!(res_err.summary.contains("unauthorized"));
}

#[test]
fn test_error_http_status_mapping() {
    assert_eq!(SkociError::unauthorized("x").http_status(), 401);
    assert_eq!(SkociError::scope_violation("a", "b").http_status(), 403);
    assert_eq!(SkociError::approval_required("r", "deploy").http_status(), 403);
    assert_eq!(SkociError::app_not_found("a").http_status(), 404);
    assert_eq!(SkociError::payload_too_large(200).http_status(), 413);
    assert_eq!(SkociError::new("something_new", "m", "f").http_status(), 500);
}

#[test]
fn test_builtin_policy_requires_approval_for_production_deploy() {
    let policy = SkociPolicy::default();
    assert_eq!(policy.preview.deploy, ActionPermission::Auto);
    assert_eq!(policy.production.deploy, ActionPermission::Approve);
    assert_eq!(policy.preview.db_restore, ActionPermission::Approve);
}

#[test]
fn test_config_defaults_and_legacy_fields() {
    let yaml = r#"
version: 1
app: belajarpaper
server: default
build:
  mode: server
run:
  port: 80
env:
  public:
    NODE_ENV: production
"#;
    let cfg: SkociConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.run.health, "/healthz");
    assert_eq!(cfg.run.health_timeout_secs, 60);
    assert_eq!(cfg.build.dockerfile, "Dockerfile");
    assert_eq!(cfg.env.public.get("NODE_ENV").map(String::as_str), Some("production"));
    assert!(cfg.services.is_empty());
}
