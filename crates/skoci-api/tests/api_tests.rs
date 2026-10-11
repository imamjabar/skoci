use skoci_api::error::SkociError;
use skoci_api::models::{ApiResponse, SkociPolicy};

#[test]
fn test_error_serialization() {
    let err = SkociError::secret_missing("DATABASE_URL");
    let json = serde_json::to_string(&err).unwrap();
    assert!(json.contains("\"code\":\"secret_missing\""));
    assert!(json.contains("Secret 'DATABASE_URL' belum diset"));
    assert!(json.contains("\"fix\":\"skoci secret set DATABASE_URL\""));

    let deserialized: SkociError = serde_json::from_str(&json).unwrap();
    assert_eq!(err, deserialized);
}

#[test]
fn test_api_response_wrapper() {
    let res = ApiResponse::ok("healthy", "System healthy");
    assert!(res.success);
    assert_eq!(res.summary, "System healthy");

    let err = SkociError::unauthorized("Token kedaluwarsa");
    let res_err = ApiResponse::<()>::err(err);
    assert!(!res_err.success);
    assert!(res_err.summary.contains("unauthorized"));
}

#[test]
fn test_policy_deserialization() {
    let yaml = r#"
version: 1
preview:
  deploy: auto
  rollback: auto
production:
  deploy: approve
  rollback: auto
"#;
    let policy: SkociPolicy = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(policy.version, 1);
}
