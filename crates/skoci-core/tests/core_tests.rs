use skoci_core::config::ConfigLoader;
use skoci_core::detector::{Framework, ProjectDetector};
use skoci_core::policy::{Action, PolicyEvaluator};
use skoci_core::state_machine::{DeploymentContext, DeploymentState};

#[test]
fn test_config_loader_validation() {
    let valid_yaml = r#"
version: 1
app: my-shop
run:
  port: 3000
domains:
  - shop.example.com
"#;
    let config = ConfigLoader::parse_config(valid_yaml).unwrap();
    assert_eq!(config.app, "my-shop");
    assert_eq!(config.run.port, 3000);

    // Invalid app name with spaces
    let invalid_app = r#"
version: 1
app: "my shop; rm -rf /"
run:
  port: 3000
"#;
    assert!(ConfigLoader::parse_config(invalid_app).is_err());
}

#[test]
fn test_policy_evaluation() {
    let policy_yaml = r#"
version: 1
preview:
  deploy: auto
  rollback: auto
production:
  deploy: approve
  rollback: auto
  db_restore: approve
  delete: approve
"#;
    let policy = ConfigLoader::parse_policy(policy_yaml).unwrap();

    // Preview deploy by agent should succeed
    assert!(PolicyEvaluator::evaluate(&policy, "preview", Action::Deploy, false).is_ok());

    // Production deploy by agent should require approval
    let err = PolicyEvaluator::evaluate(&policy, "production", Action::Deploy, false).unwrap_err();
    assert_eq!(err.code, "approval_required");

    // Production deploy by human should succeed
    assert!(PolicyEvaluator::evaluate(&policy, "production", Action::Deploy, true).is_ok());
}

#[test]
fn test_state_machine_transitions() {
    let mut ctx = DeploymentContext::new("app", "v1", "commit1", "preview");
    assert_eq!(ctx.state, DeploymentState::Idle);

    ctx.transition_to(DeploymentState::Building).unwrap();
    ctx.transition_to(DeploymentState::Deploying).unwrap();
    ctx.transition_to(DeploymentState::HealthChecking).unwrap();
    ctx.transition_to(DeploymentState::SmokeTesting).unwrap();
    ctx.transition_to(DeploymentState::Live).unwrap();

    // Invalid jump from Live directly to Building
    assert!(ctx.transition_to(DeploymentState::Building).is_err());
}

#[test]
fn test_project_detector() {
    let yaml = ProjectDetector::generate_skoci_yaml("toko", &Framework::Rust);
    assert!(yaml.contains("app: toko"));
    assert!(yaml.contains("port: 8080"));
}
