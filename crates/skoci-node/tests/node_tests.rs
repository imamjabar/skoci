use skoci_node::backup::BackupManager;
use skoci_node::secret::SecretStore;

#[test]
fn test_secret_encryption_and_redaction() {
    let mut store = SecretStore::new("test_master_seed_12345");
    store.set_secret("my-app", "DB_PASS", "super_secret_value_xyz").unwrap();

    let decrypted = store.get_secret_decrypted("my-app", "DB_PASS").unwrap();
    assert_eq!(decrypted, "super_secret_value_xyz");

    // Redaction
    let log_line = "Error connecting with password super_secret_value_xyz on port 5432";
    let redacted = store.redact_logs("my-app", log_line);
    assert!(!redacted.contains("super_secret_value_xyz"));
    assert!(redacted.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn test_backup_and_restore() {
    let temp_dir = std::env::temp_dir().join("skoci_test_backups");
    let mgr = BackupManager::new(temp_dir);

    let backup = mgr.create_backup("testapp", "db").await.unwrap();
    assert_eq!(backup.app, "testapp");
    assert!(backup.size_bytes > 0);

    let restored = mgr.restore_backup("testapp", "db", &backup.backup_id).await.unwrap();
    assert_eq!(restored.status, "restored");
}
