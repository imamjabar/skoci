use chrono::{DateTime, Utc};
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::DatabaseEngine;
use skoci_api::responses::{DbBackupResponse, DbRestoreResponse};
use std::path::PathBuf;

use crate::docker::DockerClient;
use crate::services::ServiceManager;

pub struct BackupManager {
    root: PathBuf,
    docker: DockerClient,
}

fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn parse_timestamp_from_backup_id(id: &str) -> Option<DateTime<Utc>> {
    let (_, ts_str) = id.rsplit_once('-')?;
    let naive = chrono::NaiveDateTime::parse_from_str(ts_str, "%Y%m%d%H%M%S").ok()?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

impl BackupManager {
    pub fn new(root: impl Into<PathBuf>, docker: DockerClient) -> Self {
        Self {
            root: root.into(),
            docker,
        }
    }

    /// postgres: `pg_dump -Fc` (ext `.dump`); mysql: mysqldump (ext `.sql`); valkey: `valkey-cli -a $pw SAVE` then copy `/data/dump.rdb` (ext `.rdb`).
    /// Run inside the service container via `sh -c '<fixed literal>'` that only references the container's own env vars.
    pub async fn create_backup(
        &self,
        app: &str,
        service: &str,
        engine: DatabaseEngine,
    ) -> SkociResult<DbBackupResponse> {
        if !is_valid_id(app) || !is_valid_id(service) {
            return Err(SkociError::bad_request(
                "Nama app atau service tidak valid",
                "Gunakan karakter a-z, 0-9, _, atau -",
            ));
        }

        let now = Utc::now();
        let timestamp = now.format("%Y%m%d%H%M%S").to_string();
        let backup_id = format!("{service}-{timestamp}");

        let ext = match engine {
            DatabaseEngine::Postgres => "dump",
            DatabaseEngine::Mysql => "sql",
            DatabaseEngine::Valkey => "rdb",
        };

        let dest_dir = self.root.join(app).join(service);
        tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| {
            SkociError::internal(
                format!("Gagal membuat folder backup: {e}"),
                "Periksa izin folder backup",
            )
        })?;

        let dest_file = dest_dir.join(format!("{backup_id}.{ext}"));
        let container = ServiceManager::container_name(app, service);

        match engine {
            DatabaseEngine::Postgres => {
                let argv = &[
                    "sh",
                    "-c",
                    "exec pg_dump -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -Fc",
                ];
                self.docker.exec_to_file(&container, argv, &dest_file).await?;
            }
            DatabaseEngine::Mysql => {
                let argv = &[
                    "sh",
                    "-c",
                    "exec mysqldump -u\"$MYSQL_USER\" -p\"$MYSQL_PASSWORD\" \"$MYSQL_DATABASE\"",
                ];
                self.docker.exec_to_file(&container, argv, &dest_file).await?;
            }
            DatabaseEngine::Valkey => {
                let argv = &[
                    "sh",
                    "-c",
                    "exec valkey-cli -a \"$VALKEY_PASSWORD\" SAVE",
                ];
                self.docker.exec(&container, argv).await?;
                self.docker.copy_from(&container, "/data/dump.rdb", &dest_file).await?;
            }
        }

        let meta = tokio::fs::metadata(&dest_file).await.map_err(|e| {
            SkociError::internal(
                format!("Gagal membaca metadata backup: {e}"),
                "Periksa disk",
            )
        })?;

        Ok(DbBackupResponse {
            backup_id,
            app: app.to_string(),
            service: service.to_string(),
            size_bytes: meta.len(),
            location: dest_file.to_string_lossy().to_string(),
            created_at: now,
        })
    }

    pub async fn restore_backup(
        &self,
        app: &str,
        service: &str,
        engine: DatabaseEngine,
        backup_id: &str,
    ) -> SkociResult<DbRestoreResponse> {
        if !is_valid_id(app) || !is_valid_id(service) || !is_valid_id(backup_id) {
            return Err(SkociError::bad_request(
                "Parameter ID backup tidak valid",
                "Gunakan karakter a-z, 0-9, _, atau -",
            ));
        }

        let ext = match engine {
            DatabaseEngine::Postgres => "dump",
            DatabaseEngine::Mysql => "sql",
            DatabaseEngine::Valkey => "rdb",
        };

        let src_file = self.root.join(app).join(service).join(format!("{backup_id}.{ext}"));
        if !src_file.exists() {
            return Err(SkociError::not_found(
                format!("File backup '{backup_id}' tidak ditemukan"),
                "Periksa daftar backup dengan 'skoci db list'",
            ));
        }

        let container = ServiceManager::container_name(app, service);

        match engine {
            DatabaseEngine::Postgres => {
                let argv = &[
                    "sh",
                    "-c",
                    "exec pg_restore -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" --clean --if-exists --no-owner",
                ];
                self.docker.exec_from_file(&container, argv, &src_file).await?;
            }
            DatabaseEngine::Mysql => {
                let argv = &[
                    "sh",
                    "-c",
                    "exec mysql -u\"$MYSQL_USER\" -p\"$MYSQL_PASSWORD\" \"$MYSQL_DATABASE\"",
                ];
                self.docker.exec_from_file(&container, argv, &src_file).await?;
            }
            DatabaseEngine::Valkey => {
                self.docker.stop_container(&container, 10).await?;
                self.docker.copy_to(&src_file, &container, "/data/dump.rdb").await?;
                self.docker.start_container(&container).await?;
            }
        }

        Ok(DbRestoreResponse {
            app: app.to_string(),
            service: service.to_string(),
            backup_id: backup_id.to_string(),
            status: "restored".to_string(),
            request_id: None,
        })
    }

    pub fn list_backups(&self, app: &str) -> SkociResult<Vec<DbBackupResponse>> {
        if !is_valid_id(app) {
            return Err(SkociError::bad_request(
                "Nama app tidak valid",
                "Gunakan karakter a-z, 0-9, _, atau -",
            ));
        }

        let app_dir = self.root.join(app);
        if !app_dir.exists() {
            return Ok(Vec::new());
        }

        let mut backups = Vec::new();
        let service_entries = std::fs::read_dir(&app_dir).map_err(|e| {
            SkociError::internal(
                format!("Gagal membaca direktori backup: {e}"),
                "Periksa izin folder",
            )
        })?;

        for s_entry in service_entries.flatten() {
            let s_path = s_entry.path();
            if !s_path.is_dir() {
                continue;
            }

            let service_name = match s_path.file_name().and_then(|n| n.to_str()) {
                Some(s) if is_valid_id(s) => s.to_string(),
                _ => continue,
            };

            let file_entries = match std::fs::read_dir(&s_path) {
                Ok(fe) => fe,
                Err(_) => continue,
            };

            for f_entry in file_entries.flatten() {
                let f_path = f_entry.path();
                if !f_path.is_file() {
                    continue;
                }

                let ext = f_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "dump" | "sql" | "rdb") {
                    continue;
                }

                let backup_id = match f_path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) if is_valid_id(s) => s.to_string(),
                    _ => continue,
                };

                let meta = f_entry.metadata().ok();
                let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);

                let created_at = parse_timestamp_from_backup_id(&backup_id).unwrap_or_else(|| {
                    meta.and_then(|m| m.modified().ok())
                        .map(DateTime::<Utc>::from)
                        .unwrap_or_else(Utc::now)
                });

                backups.push(DbBackupResponse {
                    backup_id,
                    app: app.to_string(),
                    service: service_name.clone(),
                    size_bytes,
                    location: f_path.to_string_lossy().to_string(),
                    created_at,
                });
            }
        }

        backups.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(backups)
    }

    pub fn prune(&self, app: &str, service: &str, keep: u32) -> SkociResult<usize> {
        if !is_valid_id(app) || !is_valid_id(service) {
            return Err(SkociError::bad_request(
                "Nama app atau service tidak valid",
                "Gunakan karakter a-z, 0-9, _, atau -",
            ));
        }

        let service_dir = self.root.join(app).join(service);
        if !service_dir.exists() {
            return Ok(0);
        }

        let file_entries = std::fs::read_dir(&service_dir).map_err(|e| {
            SkociError::internal(
                format!("Gagal membaca direktori service untuk prune: {e}"),
                "Periksa izin folder",
            )
        })?;

        let mut files: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();

        for f_entry in file_entries.flatten() {
            let f_path = f_entry.path();
            if !f_path.is_file() {
                continue;
            }

            let ext = f_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "dump" | "sql" | "rdb") {
                continue;
            }

            let backup_id = match f_path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if is_valid_id(s) => s.to_string(),
                _ => continue,
            };

            let meta = f_entry.metadata().ok();
            let created_at = parse_timestamp_from_backup_id(&backup_id).unwrap_or_else(|| {
                meta.and_then(|m| m.modified().ok())
                    .map(DateTime::<Utc>::from)
                    .unwrap_or_else(Utc::now)
            });

            files.push((f_path, created_at));
        }

        files.sort_by_key(|f| std::cmp::Reverse(f.1));

        let keep_usize = keep as usize;
        if files.len() <= keep_usize {
            return Ok(0);
        }

        let mut pruned = 0;
        for (path, _) in &files[keep_usize..] {
            if std::fs::remove_file(path).is_ok() {
                pruned += 1;
            }
        }

        Ok(pruned)
    }
}
