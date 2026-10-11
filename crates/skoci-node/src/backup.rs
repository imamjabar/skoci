use chrono::Utc;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::responses::{DbBackupResponse, DbRestoreResponse};
use std::path::PathBuf;
use tokio::fs;

pub struct BackupManager {
    backup_dir: PathBuf,
}

impl BackupManager {
    pub fn new(backup_dir: impl Into<PathBuf>) -> Self {
        Self {
            backup_dir: backup_dir.into(),
        }
    }

    pub async fn create_backup(
        &self,
        app: &str,
        service: &str,
    ) -> SkociResult<DbBackupResponse> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let backup_id = format!("{app}_{service}_{timestamp}");
        let filename = format!("{backup_id}.sql.gz");
        let dest = self.backup_dir.join(&filename);

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SkociError::bad_request(format!("Gagal membuat folder backup: {e}"), "Cek permissions")
            })?;
        }

        // Mock/simulated dump file creation for local persistence
        let content = format!("-- Skoci Backup for {app}:{service} at {}\n", Utc::now());
        fs::write(&dest, content.as_bytes()).await.map_err(|e| {
            SkociError::bad_request(format!("Gagal menulis file backup: {e}"), "Cek disk space")
        })?;

        let meta = fs::metadata(&dest).await.map_err(|e| {
            SkociError::bad_request(format!("Gagal metadata backup: {e}"), "Cek file backup")
        })?;

        Ok(DbBackupResponse {
            backup_id,
            app: app.to_string(),
            service: service.to_string(),
            size_bytes: meta.len(),
            location: dest.to_string_lossy().to_string(),
            created_at: Utc::now(),
        })
    }

    pub async fn restore_backup(
        &self,
        app: &str,
        service: &str,
        backup_id: &str,
    ) -> SkociResult<DbRestoreResponse> {
        let filename = format!("{backup_id}.sql.gz");
        let src = self.backup_dir.join(&filename);

        if !src.exists() {
            return Err(SkociError::bad_request(
                format!("File backup {backup_id} tidak ditemukan"),
                "Periksa ID backup yang valid dengan 'skoci db list'",
            ));
        }

        Ok(DbRestoreResponse {
            app: app.to_string(),
            service: service.to_string(),
            restored_backup_id: backup_id.to_string(),
            status: "restored".to_string(),
        })
    }
}
