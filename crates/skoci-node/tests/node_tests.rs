use skoci_api::models::{DatabaseEngine, ServiceConfig};
use skoci_node::backup::BackupManager;
use skoci_node::docker::{ContainerConfig, DockerClient, PortPublish};
use skoci_node::services::{ServiceManager, ServiceSpec};
use std::fs;

#[test]
fn test_backup_list_sorting_and_prune() {
    let temp_dir = std::env::temp_dir().join(format!("skoci_backup_test_{:x}", rand::random::<u64>()));
    let app_dir = temp_dir.join("testapp").join("db");
    fs::create_dir_all(&app_dir).unwrap();

    let f1 = app_dir.join("db-20261011100000.dump");
    let f2 = app_dir.join("db-20261011120000.dump");
    let f3 = app_dir.join("db-20261011140000.dump");

    fs::write(&f1, b"backup 1").unwrap();
    fs::write(&f2, b"backup 2 larger content").unwrap();
    fs::write(&f3, b"backup 3 newest").unwrap();

    let mgr = BackupManager::new(&temp_dir, DockerClient::new());

    let list = mgr.list_backups("testapp").unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].backup_id, "db-20261011140000");
    assert_eq!(list[1].backup_id, "db-20261011120000");
    assert_eq!(list[2].backup_id, "db-20261011100000");
    assert!(list[1].size_bytes > list[0].size_bytes);

    // Prune: keep 2 newest
    let pruned = mgr.prune("testapp", "db", 2).unwrap();
    assert_eq!(pruned, 1);

    let remaining = mgr.list_backups("testapp").unwrap();
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[0].backup_id, "db-20261011140000");
    assert_eq!(remaining[1].backup_id, "db-20261011120000");
    assert!(!f1.exists());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_backup_id_validation_rejection() {
    let temp_dir = std::env::temp_dir().join(format!("skoci_backup_val_{:x}", rand::random::<u64>()));
    let mgr = BackupManager::new(&temp_dir, DockerClient::new());

    // App name escaping via traversal
    let err = mgr.list_backups("../etc").unwrap_err();
    assert_eq!(err.code, "bad_request");

    // Invalid chars in prune
    let err2 = mgr.prune("test app", "db", 1).unwrap_err();
    assert_eq!(err2.code, "bad_request");

    let err3 = mgr.prune("app", "db;rm -rf", 1).unwrap_err();
    assert_eq!(err3.code, "bad_request");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_service_manager_url_and_password() {
    let pw = ServiceManager::generate_password();
    assert_eq!(pw.len(), 32);
    assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));

    let cfg_pg = ServiceConfig {
        engine: DatabaseEngine::Postgres,
        version: "16".to_string(),
        memory: None,
        backup: None,
    };
    let spec_pg = ServiceSpec {
        app: "my-cool-app",
        service: "db",
        cfg: &cfg_pg,
        password: "secretpassword123",
    };

    let url_pg = ServiceManager::connection_url(&spec_pg);
    assert_eq!(
        url_pg,
        "postgresql://skoci:secretpassword123@db:5432/my_cool_app"
    );

    let cfg_valkey = ServiceConfig {
        engine: DatabaseEngine::Valkey,
        version: "8.0".to_string(),
        memory: None,
        backup: None,
    };
    let spec_valkey = ServiceSpec {
        app: "my-cool-app",
        service: "cache",
        cfg: &cfg_valkey,
        password: "valkeypassword",
    };
    let url_valkey = ServiceManager::connection_url(&spec_valkey);
    assert_eq!(url_valkey, "redis://:valkeypassword@cache:6379");

    assert_eq!(ServiceManager::env_var_name("db"), "DATABASE_URL");
    assert_eq!(ServiceManager::env_var_name("redis-cache"), "REDIS_CACHE_URL");
    assert_eq!(ServiceManager::password_secret_key("db"), "SKOCI_DB_PASSWORD");
    assert_eq!(
        ServiceManager::password_secret_key("redis-cache"),
        "SKOCI_REDIS_CACHE_PASSWORD"
    );
}

#[test]
fn test_readiness_failure_redacts_password_and_url() {
    let cfg = ServiceConfig {
        engine: DatabaseEngine::Postgres,
        version: "16".to_string(),
        memory: None,
        backup: None,
    };
    let spec = ServiceSpec {
        app: "test-app",
        service: "db",
        cfg: &cfg,
        password: "supersecretpassword123",
    };

    let url = ServiceManager::connection_url(&spec);
    let sample_logs = format!(
        "2026-10-11T12:00:00Z FATAL: password authentication failed for user \"skoci\" with password \"supersecretpassword123\"\nConnected to {url}\nFailed to connect"
    );

    let err = ServiceManager::format_readiness_error(&spec, &sample_logs);
    assert!(!err.message.contains("supersecretpassword123"));
    assert!(!err.message.contains(&url));
    assert!(err.message.contains("[REDACTED]"));
    assert!(err.message.contains("Database 'db' failed to become ready"));
}

#[test]
fn test_container_config_pids_limit() {
    let cfg = ContainerConfig {
        pids_limit: Some(512),
        ..Default::default()
    };
    assert_eq!(cfg.pids_limit, Some(512));
}

#[tokio::test]
#[ignore = "Requires running Docker daemon"]
async fn test_docker_daemon_integration() {
    let client = DockerClient::new();
    let available = DockerClient::check_daemon().await;
    if !available {
        return;
    }

    let version = client.version().await.unwrap();
    assert!(!version.is_empty());

    let state = client.inspect("non_existent_container_skoci_test").await.unwrap();
    assert!(!state.exists);
    assert_eq!(state.status, "missing");

    let cfg = ContainerConfig {
        name: "skoci-test-dummy".to_string(),
        image: "alpine:latest".to_string(),
        publish: vec![PortPublish {
            host_ip: "127.0.0.1".to_string(),
            host_port: None,
            container_port: 80,
        }],
        command: vec!["sleep".to_string(), "10".to_string()],
        ..Default::default()
    };
    let _ = client.remove_container(&cfg.name).await;
}
