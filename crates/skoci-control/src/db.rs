use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRecord {
    pub name: String,
    pub server: String,
    pub live_version: Option<String>,
    pub domains: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentRecord {
    pub id: String,
    pub app: String,
    pub version: String,
    pub commit_hash: String,
    pub env: String,
    pub status: String,
    pub evidence: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub id: String,
    pub app: String,
    pub action: String,
    pub target_env: String,
    pub reason: String,
    pub status: String,
    pub requested_by: String,
    pub approved_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: i64,
    pub timestamp: String,
    pub actor: String,
    pub actor_type: String,
    pub action: String,
    pub target: String,
    pub details: String,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open_in_memory() -> SkociResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            SkociError::bad_request(format!("Gagal membuka SQLite in-memory: {e}"), "Coba lagi")
        })?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open(path: impl AsRef<Path>) -> SkociResult<Self> {
        let conn = Connection::open(path).map_err(|e| {
            SkociError::bad_request(format!("Gagal membuka SQLite: {e}"), "Periksa izin file database")
        })?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&self) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS apps (
                name TEXT PRIMARY KEY,
                server TEXT NOT NULL,
                live_version TEXT,
                domains TEXT,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS deployments (
                id TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                version TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                env TEXT NOT NULL,
                status TEXT NOT NULL,
                evidence TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS approvals (
                id TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                action TEXT NOT NULL,
                target_env TEXT NOT NULL,
                reason TEXT NOT NULL,
                status TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                approved_by TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                actor TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                action TEXT NOT NULL,
                target TEXT NOT NULL,
                details TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS secrets (
                app TEXT NOT NULL,
                key TEXT NOT NULL,
                ciphertext_hex TEXT NOT NULL,
                nonce_hex TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (app, key)
            );

            CREATE TABLE IF NOT EXISTS agent_tokens (
                token_hash TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| {
            SkociError::bad_request(format!("Migrasi database gagal: {e}"), "Periksa skema DB")
        })?;
        Ok(())
    }

    pub fn upsert_app(
        &self,
        name: &str,
        server: &str,
        live_version: Option<&str>,
        domains: &[String],
    ) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        let domains_json = serde_json::to_string(domains).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            r#"
            INSERT INTO apps (name, server, live_version, domains, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(name) DO UPDATE SET
                server = excluded.server,
                live_version = coalesce(excluded.live_version, apps.live_version),
                domains = excluded.domains,
                updated_at = excluded.updated_at
            "#,
            params![name, server, live_version, domains_json, now],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal upsert app: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn get_app(&self, name: &str) -> SkociResult<Option<AppRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name, server, live_version, domains, updated_at FROM apps WHERE name = ?1")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![name])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            let domains_json: String = row.get(3).unwrap_or_else(|_| "[]".to_string());
            let domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
            Ok(Some(AppRecord {
                name: row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                server: row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                live_version: row.get(2).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                domains,
                updated_at: row.get(4).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_apps(&self) -> SkociResult<Vec<AppRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name, server, live_version, domains, updated_at FROM apps ORDER BY name ASC")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let rows = stmt
            .query_map([], |row| {
                let domains_json: String = row.get(3).unwrap_or_else(|_| "[]".to_string());
                let domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
                Ok(AppRecord {
                    name: row.get(0)?,
                    server: row.get(1)?,
                    live_version: row.get(2)?,
                    domains,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        let mut apps = Vec::new();
        for app in rows {
            apps.push(app.map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?);
        }
        Ok(apps)
    }

    pub fn save_deployment(&self, dep: &DeploymentRecord) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO deployments (id, app, version, commit_hash, env, status, evidence, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                dep.id,
                dep.app,
                dep.version,
                dep.commit_hash,
                dep.env,
                dep.status,
                dep.evidence,
                dep.created_at
            ],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal simpan deployment: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn get_latest_deployment(&self, app: &str) -> SkociResult<Option<DeploymentRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, app, version, commit_hash, env, status, evidence, created_at FROM deployments WHERE app = ?1 ORDER BY created_at DESC LIMIT 1"
            )
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![app])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            Ok(Some(DeploymentRecord {
                id: row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                app: row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                version: row.get(2).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                commit_hash: row.get(3).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                env: row.get(4).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                status: row.get(5).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                evidence: row.get(6).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                created_at: row.get(7).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_latest_global_deployment(&self) -> SkociResult<Option<DeploymentRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, app, version, commit_hash, env, status, evidence, created_at FROM deployments ORDER BY created_at DESC LIMIT 1"
            )
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            Ok(Some(DeploymentRecord {
                id: row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                app: row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                version: row.get(2).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                commit_hash: row.get(3).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                env: row.get(4).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                status: row.get(5).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                evidence: row.get(6).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                created_at: row.get(7).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_previous_deployment(&self, app: &str) -> SkociResult<Option<DeploymentRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, app, version, commit_hash, env, status, evidence, created_at FROM deployments WHERE app = ?1 ORDER BY created_at DESC LIMIT 1 OFFSET 1"
            )
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![app])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            Ok(Some(DeploymentRecord {
                id: row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                app: row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                version: row.get(2).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                commit_hash: row.get(3).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                env: row.get(4).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                status: row.get(5).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                evidence: row.get(6).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
                created_at: row.get(7).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn save_encrypted_secret(
        &self,
        app: &str,
        key: &str,
        ciphertext_hex: &str,
        nonce_hex: &str,
    ) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT INTO secrets (app, key, ciphertext_hex, nonce_hex, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(app, key) DO UPDATE SET
                ciphertext_hex = excluded.ciphertext_hex,
                nonce_hex = excluded.nonce_hex,
                updated_at = excluded.updated_at
            "#,
            params![app, key, ciphertext_hex, nonce_hex, now],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal simpan secret: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn list_secret_keys(&self, app: &str) -> SkociResult<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT key FROM secrets WHERE app = ?1 ORDER BY key ASC")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let rows = stmt
            .query_map(params![app], |row| row.get(0))
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        let mut keys = Vec::new();
        for key in rows {
            keys.push(key.map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?);
        }
        Ok(keys)
    }

    pub fn get_encrypted_secret(&self, app: &str, key: &str) -> SkociResult<Option<(String, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT ciphertext_hex, nonce_hex FROM secrets WHERE app = ?1 AND key = ?2")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![app, key])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            let ciphertext: String = row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
            let nonce: String = row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
            Ok(Some((ciphertext, nonce)))
        } else {
            Ok(None)
        }
    }

    pub fn record_audit(
        &self,
        actor: &str,
        actor_type: &str,
        action: &str,
        target: &str,
        details: &str,
    ) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO audit_logs (timestamp, actor, actor_type, action, target, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now, actor, actor_type, action, target, details],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal menulis audit: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn list_audit_logs(&self, limit: usize) -> SkociResult<Vec<AuditRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT id, timestamp, actor, actor_type, action, target, details FROM audit_logs ORDER BY id DESC LIMIT ?1")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(AuditRecord {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    actor: row.get(2)?,
                    actor_type: row.get(3)?,
                    action: row.get(4)?,
                    target: row.get(5)?,
                    details: row.get(6)?,
                })
            })
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        let mut logs = Vec::new();
        for log in rows {
            logs.push(log.map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?);
        }
        Ok(logs)
    }

    pub fn create_approval(
        &self,
        id: &str,
        app: &str,
        action: &str,
        target_env: &str,
        reason: &str,
        requested_by: &str,
    ) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO approvals (id, app, action, target_env, reason, status, requested_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
            params![id, app, action, target_env, reason, requested_by, now],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal membuat approval: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn approve_request(&self, id: &str, approved_by: &str) -> SkociResult<bool> {
        let conn = self.conn.lock();
        let rows = conn.execute(
            "UPDATE approvals SET status = 'approved', approved_by = ?1 WHERE id = ?2 AND status = 'pending'",
            params![approved_by, id],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal menyetujui approval: {e}"), "Coba lagi"))?;
        Ok(rows > 0)
    }

    pub fn reject_request(&self, id: &str, rejected_by: &str) -> SkociResult<bool> {
        let conn = self.conn.lock();
        let rows = conn.execute(
            "UPDATE approvals SET status = 'rejected', approved_by = ?1 WHERE id = ?2 AND status = 'pending'",
            params![rejected_by, id],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal menolak approval: {e}"), "Coba lagi"))?;
        Ok(rows > 0)
    }

    pub fn get_approval_status(&self, id: &str) -> SkociResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT status FROM approvals WHERE id = ?1")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![id])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            let status: String = row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    pub fn list_pending_approvals(&self) -> SkociResult<Vec<ApprovalRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, app, action, target_env, reason, status, requested_by, approved_by, created_at FROM approvals WHERE status = 'pending' ORDER BY created_at DESC"
            )
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ApprovalRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    action: row.get(2)?,
                    target_env: row.get(3)?,
                    reason: row.get(4)?,
                    status: row.get(5)?,
                    requested_by: row.get(6)?,
                    approved_by: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        let mut list = Vec::new();
        for item in rows {
            list.push(item.map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?);
        }
        Ok(list)
    }

    pub fn save_agent_token(&self, token_hash: &str, app: &str, name: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO agent_tokens (token_hash, app, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, app, name, now],
        )
        .map_err(|e| SkociError::bad_request(format!("Gagal simpan token: {e}"), "Coba lagi"))?;
        Ok(())
    }

    pub fn get_agent_token_scope(&self, token_hash: &str) -> SkociResult<Option<(String, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, name FROM agent_tokens WHERE token_hash = ?1")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let mut rows = stmt
            .query(params![token_hash])
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        if let Some(row) = rows.next().map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))? {
            let app: String = row.get(0).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
            let name: String = row.get(1).map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
            Ok(Some((app, name)))
        } else {
            Ok(None)
        }
    }

    pub fn list_agent_tokens(&self, app: &str) -> SkociResult<Vec<(String, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name, created_at FROM agent_tokens WHERE app = ?1 ORDER BY created_at DESC")
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;
        let rows = stmt
            .query_map(params![app], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?;

        let mut list = Vec::new();
        for item in rows {
            list.push(item.map_err(|e| SkociError::bad_request(e.to_string(), "Coba lagi"))?);
        }
        Ok(list)
    }
}
