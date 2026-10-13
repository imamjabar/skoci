use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{SkociConfig, SkociEvidence};
use skoci_api::responses::{ApprovalInfo, AuditEntry, DeploymentInfo};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AppRecord {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DeploymentRecord {
    pub id: String,
    pub app: String,
    pub env: String,
    pub version: String,
    pub kind: String,
    pub status: String,
    pub commit_hash: String,
    pub image: String,
    pub container: String,
    pub host_port: Option<u16>,
    pub triggered_by: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error_json: Option<String>,
    pub evidence_json: Option<String>,
}

impl DeploymentRecord {
    pub fn to_info(&self) -> DeploymentInfo {
        let error = self
            .error_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<SkociError>(s).ok());
        let evidence = self
            .evidence_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<SkociEvidence>(s).ok());

        DeploymentInfo {
            id: self.id.clone(),
            app: self.app.clone(),
            env: self.env.clone(),
            version: self.version.clone(),
            kind: self.kind.clone(),
            status: self.status.clone(),
            triggered_by: self.triggered_by.clone(),
            created_at: self.created_at,
            finished_at: self.finished_at,
            error,
            evidence,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteRecord {
    pub app: String,
    pub env: String,
    pub container: String,
    pub host_port: u16,
    pub version: String,
    pub hosts: Vec<String>,
    pub deployed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SecretRecord {
    pub app: String,
    pub key: String,
    pub ciphertext_hex: String,
    pub nonce_hex: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AgentTokenRecord {
    pub token_hash: String,
    pub app: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    pub id: String,
    pub app: String,
    pub action: String,
    pub detail: String,
    pub reason: String,
    pub status: String,
    pub requested_by: String,
    pub decided_by: Option<String>,
    pub payload_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ApprovalRecord {
    pub fn to_info(&self) -> ApprovalInfo {
        ApprovalInfo {
            id: self.id.clone(),
            app: self.app.clone(),
            action: self.action.clone(),
            detail: self.detail.clone(),
            reason: self.reason.clone(),
            status: self.status.clone(),
            requested_by: self.requested_by.clone(),
            decided_by: self.decided_by.clone(),
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub deployments_keep_per_app: usize,
    pub deployments_max_age_days: Option<u32>,
    pub approvals_max_age_days: u32,
    pub audit_logs_max_age_days: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub pruned_deployments: usize,
    pub pruned_approvals: usize,
    pub pruned_audit_logs: usize,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open_in_memory() -> SkociResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            SkociError::internal(format!("Failed to open SQLite in-memory: {e}"), "Try again")
        })?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open(path: impl AsRef<Path>) -> SkociResult<Self> {
        let conn = Connection::open(path).map_err(|e| {
            SkociError::internal(format!("Failed to open SQLite: {e}"), "Check database file permissions")
        })?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&self) -> SkociResult<()> {
        let conn = self.conn.lock();

        // Skoci <= 0.1 stored fabricated deployments/apps, secrets encrypted with a hardcoded seed,
        // and exposed raw agent tokens through a public audit endpoint. None of it is trustworthy,
        // so it is dropped once and every old token is revoked.
        if Self::table_columns(&conn, "apps")?.contains("server") {
            tracing::warn!(
                "Legacy Skoci database detected: old data removed and all legacy agent tokens revoked. Create a new token with 'skoci token create' and reset secrets with 'skoci secret set'"
            );
            conn.execute_batch(
                "DROP TABLE IF EXISTS apps; DROP TABLE IF EXISTS deployments; DROP TABLE IF EXISTS approvals; DROP TABLE IF EXISTS secrets; DROP TABLE IF EXISTS agent_tokens;",
            )
            .map_err(|e| SkociError::internal(format!("Legacy data migration failed: {e}"), "Check database file permissions"))?;
        }

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS apps (
                name TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS deployments (
                id TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                env TEXT NOT NULL,
                version TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'deploy',
                status TEXT NOT NULL,
                commit_hash TEXT NOT NULL DEFAULT '',
                image TEXT NOT NULL DEFAULT '',
                container TEXT NOT NULL DEFAULT '',
                host_port INTEGER,
                triggered_by TEXT NOT NULL DEFAULT 'unknown',
                created_at TEXT NOT NULL,
                finished_at TEXT,
                error_json TEXT,
                evidence_json TEXT
            );

            CREATE TABLE IF NOT EXISTS routes (
                app TEXT NOT NULL,
                env TEXT NOT NULL,
                container TEXT NOT NULL,
                host_port INTEGER NOT NULL,
                version TEXT NOT NULL,
                hosts_json TEXT NOT NULL,
                deployed_at TEXT NOT NULL,
                PRIMARY KEY (app, env)
            );

            CREATE TABLE IF NOT EXISTS app_configs (
                app TEXT PRIMARY KEY,
                config_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
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
                created_at TEXT NOT NULL,
                last_used_at TEXT
            );

            CREATE TABLE IF NOT EXISTS approvals (
                id TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '',
                reason TEXT NOT NULL,
                status TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                decided_by TEXT,
                payload_json TEXT,
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

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| {
            SkociError::internal(format!("Database table migration failed: {e}"), "Check DB schema")
        })?;

        // Additive columns for backwards-compatibility
        Self::ensure_column(&conn, "deployments", "kind", "TEXT NOT NULL DEFAULT 'deploy'")?;
        Self::ensure_column(&conn, "deployments", "image", "TEXT NOT NULL DEFAULT ''")?;
        Self::ensure_column(&conn, "deployments", "container", "TEXT NOT NULL DEFAULT ''")?;
        Self::ensure_column(&conn, "deployments", "host_port", "INTEGER")?;
        Self::ensure_column(&conn, "deployments", "triggered_by", "TEXT NOT NULL DEFAULT 'unknown'")?;
        Self::ensure_column(&conn, "deployments", "finished_at", "TEXT")?;
        Self::ensure_column(&conn, "deployments", "error_json", "TEXT")?;
        Self::ensure_column(&conn, "deployments", "evidence_json", "TEXT")?;

        Self::ensure_column(&conn, "approvals", "detail", "TEXT NOT NULL DEFAULT ''")?;
        Self::ensure_column(&conn, "approvals", "payload_json", "TEXT")?;
        Self::ensure_column(&conn, "agent_tokens", "last_used_at", "TEXT")?;

        // Startup migration hygiene: scrub legacy audit rows whose actor starts with sk_
        conn.execute(
            "UPDATE audit_logs SET actor = 'agent:redacted' WHERE actor LIKE 'sk_%'",
            [],
        )
        .map_err(|e| SkociError::internal(format!("Scrub audit logs failed: {e}"), "Try again"))?;

        Ok(())
    }

    fn table_columns(conn: &Connection, table: &str) -> SkociResult<HashSet<String>> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        Ok(rows.flatten().collect())
    }

    fn ensure_column(conn: &Connection, table: &str, column: &str, col_def: &str) -> SkociResult<()> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut existing = HashSet::new();
        for c in rows.flatten() {
            existing.insert(c);
        }

        if !existing.contains(column) {
            let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {col_def}");
            conn.execute(&sql, [])
                .map_err(|e| SkociError::internal(format!("Alter table {table} failed: {e}"), "Try again"))?;
        }

        Ok(())
    }

    // --- Apps ---

    pub fn ensure_app(&self, name: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT INTO apps (name, created_at, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(name) DO UPDATE SET updated_at = excluded.updated_at
            "#,
            params![name, now, now],
        )
        .map_err(|e| SkociError::internal(format!("Failed to ensure app: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn list_apps(&self) -> SkociResult<Vec<AppRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name, created_at, updated_at FROM apps ORDER BY name ASC")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let created_at_str: String = row.get(1)?;
                let updated_at_str: String = row.get(2)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(AppRecord {
                    name,
                    created_at,
                    updated_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut apps = Vec::new();
        for app in rows {
            apps.push(app.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(apps)
    }

    pub fn delete_app(&self, app: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM apps WHERE name = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM routes WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM app_configs WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM secrets WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM agent_tokens WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM approvals WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        conn.execute("DELETE FROM deployments WHERE app = ?1", params![app])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        // If default app is this app, unset it
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key = 'default_app'")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        let default_val: Option<String> = stmt.query_row([], |r| r.get(0)).ok();
        if default_val.as_deref() == Some(app) {
            let _ = conn.execute("DELETE FROM settings WHERE key = 'default_app'", []);
        }

        Ok(())
    }

    // --- Deployments ---

    pub fn insert_deployment(&self, rec: &DeploymentRecord) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO deployments (
                id, app, env, version, kind, status, commit_hash, image,
                container, host_port, triggered_by, created_at, finished_at,
                error_json, evidence_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
            params![
                rec.id,
                rec.app,
                rec.env,
                rec.version,
                rec.kind,
                rec.status,
                rec.commit_hash,
                rec.image,
                rec.container,
                rec.host_port.map(|p| p as i64),
                rec.triggered_by,
                rec.created_at.to_rfc3339(),
                rec.finished_at.map(|t| t.to_rfc3339()),
                rec.error_json,
                rec.evidence_json,
            ],
        )
        .map_err(|e| SkociError::internal(format!("Failed to insert deployment: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn update_deployment(&self, rec: &DeploymentRecord) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            UPDATE deployments SET
                status = ?1,
                finished_at = ?2,
                error_json = ?3,
                evidence_json = ?4,
                container = ?5,
                host_port = ?6
            WHERE id = ?7
            "#,
            params![
                rec.status,
                rec.finished_at.map(|t| t.to_rfc3339()),
                rec.error_json,
                rec.evidence_json,
                rec.container,
                rec.host_port.map(|p| p as i64),
                rec.id,
            ],
        )
        .map_err(|e| SkociError::internal(format!("Failed to update deployment: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn get_deployment(&self, id: &str) -> SkociResult<Option<DeploymentRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT
                    id, app, env, version, kind, status, commit_hash, image,
                    container, host_port, triggered_by, created_at, finished_at,
                    error_json, evidence_json
                FROM deployments WHERE id = ?1
                "#,
            )
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![id], |row| {
                let host_port_i64: Option<i64> = row.get(9).ok();
                let created_at_str: String = row.get(11)?;
                let finished_at_str: Option<String> = row.get(12).ok();

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let finished_at = finished_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                Ok(DeploymentRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    env: row.get(2)?,
                    version: row.get(3)?,
                    kind: row.get(4)?,
                    status: row.get(5)?,
                    commit_hash: row.get(6)?,
                    image: row.get(7)?,
                    container: row.get(8)?,
                    host_port: host_port_i64.map(|p| p as u16),
                    triggered_by: row.get(10)?,
                    created_at,
                    finished_at,
                    error_json: row.get(13).ok(),
                    evidence_json: row.get(14).ok(),
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn list_deployments(&self, app: Option<&str>, limit: usize) -> SkociResult<Vec<DeploymentRecord>> {
        let conn = self.conn.lock();
        let (sql, params_vec): (String, Vec<rusqlite::types::Value>) = match app {
            Some(a) => (
                format!(
                    r#"
                    SELECT
                        id, app, env, version, kind, status, commit_hash, image,
                        container, host_port, triggered_by, created_at, finished_at,
                        error_json, evidence_json
                    FROM deployments WHERE app = ?1 ORDER BY created_at DESC LIMIT {limit}
                    "#
                ),
                vec![a.to_string().into()],
            ),
            None => (
                format!(
                    r#"
                    SELECT
                        id, app, env, version, kind, status, commit_hash, image,
                        container, host_port, triggered_by, created_at, finished_at,
                        error_json, evidence_json
                    FROM deployments ORDER BY created_at DESC LIMIT {limit}
                    "#
                ),
                vec![],
            ),
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(&params_slice[..], |row| {
                let host_port_i64: Option<i64> = row.get(9).ok();
                let created_at_str: String = row.get(11)?;
                let finished_at_str: Option<String> = row.get(12).ok();

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let finished_at = finished_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                Ok(DeploymentRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    env: row.get(2)?,
                    version: row.get(3)?,
                    kind: row.get(4)?,
                    status: row.get(5)?,
                    commit_hash: row.get(6)?,
                    image: row.get(7)?,
                    container: row.get(8)?,
                    host_port: host_port_i64.map(|p| p as u16),
                    triggered_by: row.get(10)?,
                    created_at,
                    finished_at,
                    error_json: row.get(13).ok(),
                    evidence_json: row.get(14).ok(),
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn get_latest_successful_deployment(
        &self,
        app: &str,
        env: &str,
        exclude_version: Option<&str>,
    ) -> SkociResult<Option<DeploymentRecord>> {
        let conn = self.conn.lock();
        let sql = match exclude_version {
            Some(v) => format!(
                r#"
                SELECT
                    id, app, env, version, kind, status, commit_hash, image,
                    container, host_port, triggered_by, created_at, finished_at,
                    error_json, evidence_json
                FROM deployments
                WHERE app = ?1 AND env = ?2 AND status IN ('live', 'superseded') AND version != '{v}'
                ORDER BY created_at DESC LIMIT 1
                "#
            ),
            None => r#"
                SELECT
                    id, app, env, version, kind, status, commit_hash, image,
                    container, host_port, triggered_by, created_at, finished_at,
                    error_json, evidence_json
                FROM deployments
                WHERE app = ?1 AND env = ?2 AND status IN ('live', 'superseded')
                ORDER BY created_at DESC LIMIT 1
                "#
            .to_string(),
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![app, env], |row| {
                let host_port_i64: Option<i64> = row.get(9).ok();
                let created_at_str: String = row.get(11)?;
                let finished_at_str: Option<String> = row.get(12).ok();

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let finished_at = finished_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                Ok(DeploymentRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    env: row.get(2)?,
                    version: row.get(3)?,
                    kind: row.get(4)?,
                    status: row.get(5)?,
                    commit_hash: row.get(6)?,
                    image: row.get(7)?,
                    container: row.get(8)?,
                    host_port: host_port_i64.map(|p| p as u16),
                    triggered_by: row.get(10)?,
                    created_at,
                    finished_at,
                    error_json: row.get(13).ok(),
                    evidence_json: row.get(14).ok(),
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn get_estimated_deploy_duration(&self, app: &str, env: &str) -> SkociResult<Option<u64>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT created_at, finished_at, evidence_json
                FROM deployments
                WHERE app = ?1 AND env = ?2 AND status IN ('live', 'degraded') AND finished_at IS NOT NULL
                ORDER BY created_at DESC
                LIMIT 5
                "#,
            )
            .map_err(|e| SkociError::internal(format!("Failed to prepare query: {e}"), "Try again"))?;

        let rows = stmt
            .query_map(params![app, env], |row| {
                let created: String = row.get(0)?;
                let finished: Option<String> = row.get(1)?;
                let evidence: Option<String> = row.get(2)?;
                Ok((created, finished, evidence))
            })
            .map_err(|e| SkociError::internal(format!("Failed to execute query: {e}"), "Try again"))?;

        let mut durations = Vec::new();
        for row in rows {
            let (created_str, finished_str, evidence_str) = row
                .map_err(|e| SkociError::internal(format!("Failed to read row: {e}"), "Try again"))?;

            if let Some(ev_str) = evidence_str {
                if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&ev_str) {
                    if let Some(ms) = ev.get("duration_ms").and_then(|v| v.as_u64()) {
                        durations.push(ms / 1000);
                        continue;
                    }
                }
            }

            if let Some(fin_str) = finished_str {
                if let (Ok(created), Ok(finished)) = (
                    DateTime::parse_from_rfc3339(&created_str),
                    DateTime::parse_from_rfc3339(&fin_str),
                ) {
                    let diff = (finished - created).num_seconds();
                    if diff >= 0 {
                        durations.push(diff as u64);
                    }
                }
            }
        }

        if durations.is_empty() {
            Ok(None)
        } else {
            let sum: u64 = durations.iter().sum();
            Ok(Some(sum / durations.len() as u64))
        }
    }

    pub fn mean_deployment_duration_secs(&self, app: &str, env: &str) -> SkociResult<Option<u64>> {
        self.get_estimated_deploy_duration(app, env)
    }

    // --- Routes ---

    pub fn upsert_route(&self, rec: &RouteRecord) -> SkociResult<()> {
        let conn = self.conn.lock();
        let hosts_json = serde_json::to_string(&rec.hosts).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            r#"
            INSERT INTO routes (app, env, container, host_port, version, hosts_json, deployed_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(app, env) DO UPDATE SET
                container = excluded.container,
                host_port = excluded.host_port,
                version = excluded.version,
                hosts_json = excluded.hosts_json,
                deployed_at = excluded.deployed_at
            "#,
            params![
                rec.app,
                rec.env,
                rec.container,
                rec.host_port as i64,
                rec.version,
                hosts_json,
                rec.deployed_at.to_rfc3339()
            ],
        )
        .map_err(|e| SkociError::internal(format!("Failed to upsert route: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn quick_check(&self) -> SkociResult<bool> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("PRAGMA quick_check")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        let result: Result<String, _> = stmt.query_row([], |row| row.get(0));
        Ok(result.map(|s| s == "ok").unwrap_or(false))
    }

    pub fn get_route(&self, app: &str, env: &str) -> SkociResult<Option<RouteRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT app, env, container, host_port, version, hosts_json, deployed_at FROM routes WHERE app = ?1 AND env = ?2",
            )
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![app, env], |row| {
                let hosts_json: String = row.get(5)?;
                let hosts: Vec<String> = serde_json::from_str(&hosts_json).unwrap_or_default();
                let deployed_at_str: String = row.get(6)?;
                let deployed_at = DateTime::parse_from_rfc3339(&deployed_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                let port_i64: i64 = row.get(3)?;
                Ok(RouteRecord {
                    app: row.get(0)?,
                    env: row.get(1)?,
                    container: row.get(2)?,
                    host_port: port_i64 as u16,
                    version: row.get(4)?,
                    hosts,
                    deployed_at,
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn list_routes(&self) -> SkociResult<Vec<RouteRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, env, container, host_port, version, hosts_json, deployed_at FROM routes")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map([], |row| {
                let hosts_json: String = row.get(5)?;
                let hosts: Vec<String> = serde_json::from_str(&hosts_json).unwrap_or_default();
                let deployed_at_str: String = row.get(6)?;
                let deployed_at = DateTime::parse_from_rfc3339(&deployed_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let port_i64: i64 = row.get(3)?;
                Ok(RouteRecord {
                    app: row.get(0)?,
                    env: row.get(1)?,
                    container: row.get(2)?,
                    host_port: port_i64 as u16,
                    version: row.get(4)?,
                    hosts,
                    deployed_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn list_routes_for_app(&self, app: &str) -> SkociResult<Vec<RouteRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, env, container, host_port, version, hosts_json, deployed_at FROM routes WHERE app = ?1")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map(params![app], |row| {
                let hosts_json: String = row.get(5)?;
                let hosts: Vec<String> = serde_json::from_str(&hosts_json).unwrap_or_default();
                let deployed_at_str: String = row.get(6)?;
                let deployed_at = DateTime::parse_from_rfc3339(&deployed_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let port_i64: i64 = row.get(3)?;
                Ok(RouteRecord {
                    app: row.get(0)?,
                    env: row.get(1)?,
                    container: row.get(2)?,
                    host_port: port_i64 as u16,
                    version: row.get(4)?,
                    hosts,
                    deployed_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn remove_route(&self, app: &str, env: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM routes WHERE app = ?1 AND env = ?2", params![app, env])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        Ok(())
    }

    // --- App Configs ---

    pub fn save_app_config(&self, app: &str, cfg: &SkociConfig) -> SkociResult<()> {
        let conn = self.conn.lock();
        let json = serde_json::to_string(cfg).map_err(|e| {
            SkociError::internal(format!("Failed to serialize skoci config: {e}"), "Try again")
        })?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT INTO app_configs (app, config_json, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(app) DO UPDATE SET
                config_json = excluded.config_json,
                updated_at = excluded.updated_at
            "#,
            params![app, json, now],
        )
        .map_err(|e| SkociError::internal(format!("Failed to save app config: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn get_app_config(&self, app: &str) -> SkociResult<Option<SkociConfig>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT config_json FROM app_configs WHERE app = ?1")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![app], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        if let Some(json) = opt {
            let cfg: SkociConfig = serde_json::from_str(&json).map_err(|e| {
                SkociError::internal(format!("Failed to parse app config from DB: {e}"), "Try again")
            })?;
            Ok(Some(cfg))
        } else {
            Ok(None)
        }
    }

    pub fn list_app_configs(&self) -> SkociResult<Vec<(String, SkociConfig)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, config_json FROM app_configs")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map([], |row| {
                let app: String = row.get(0)?;
                let json: String = row.get(1)?;
                Ok((app, json))
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            let (app, json) = r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
            if let Ok(cfg) = serde_json::from_str::<SkociConfig>(&json) {
                list.push((app, cfg));
            }
        }
        Ok(list)
    }

    // --- Secrets ---

    pub fn set_secret(
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
        .map_err(|e| SkociError::internal(format!("Failed to save secret: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn get_secret(&self, app: &str, key: &str) -> SkociResult<Option<SecretRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, key, ciphertext_hex, nonce_hex, updated_at FROM secrets WHERE app = ?1 AND key = ?2")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![app, key], |row| {
                let updated_at_str: String = row.get(4)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(SecretRecord {
                    app: row.get(0)?,
                    key: row.get(1)?,
                    ciphertext_hex: row.get(2)?,
                    nonce_hex: row.get(3)?,
                    updated_at,
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn list_secrets_for_app(&self, app: &str) -> SkociResult<Vec<SecretRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT app, key, ciphertext_hex, nonce_hex, updated_at FROM secrets WHERE app = ?1 ORDER BY key ASC")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map(params![app], |row| {
                let updated_at_str: String = row.get(4)?;
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(SecretRecord {
                    app: row.get(0)?,
                    key: row.get(1)?,
                    ciphertext_hex: row.get(2)?,
                    nonce_hex: row.get(3)?,
                    updated_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn list_secret_keys_for_app(&self, app: &str) -> SkociResult<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT key FROM secrets WHERE app = ?1 ORDER BY key ASC")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map(params![app], |row| row.get::<_, String>(0))
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    // --- Agent Tokens ---

    pub fn insert_agent_token(&self, token_hash: &str, app: &str, name: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO agent_tokens (token_hash, app, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, app, name, now],
        )
        .map_err(|e| SkociError::internal(format!("Failed to save token: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn get_agent_token_by_hash(&self, token_hash: &str) -> SkociResult<Option<AgentTokenRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT token_hash, app, name, created_at, last_used_at FROM agent_tokens WHERE token_hash = ?1")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![token_hash], |row| {
                let created_at_str: String = row.get(3)?;
                let last_used_at_str: Option<String> = row.get(4).ok();

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let last_used_at = last_used_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                Ok(AgentTokenRecord {
                    token_hash: row.get(0)?,
                    app: row.get(1)?,
                    name: row.get(2)?,
                    created_at,
                    last_used_at,
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn update_token_last_used_at(&self, token_hash: &str, ts: DateTime<Utc>) -> SkociResult<()> {
        let conn = self.conn.lock();
        let _ = conn.execute(
            "UPDATE agent_tokens SET last_used_at = ?1 WHERE token_hash = ?2",
            params![ts.to_rfc3339(), token_hash],
        );
        Ok(())
    }

    pub fn list_agent_tokens_for_app(&self, app: &str) -> SkociResult<Vec<AgentTokenRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT token_hash, app, name, created_at, last_used_at FROM agent_tokens WHERE app = ?1 ORDER BY created_at DESC")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map(params![app], |row| {
                let created_at_str: String = row.get(3)?;
                let last_used_at_str: Option<String> = row.get(4).ok();
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let last_used_at = last_used_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });
                Ok(AgentTokenRecord {
                    token_hash: row.get(0)?,
                    app: row.get(1)?,
                    name: row.get(2)?,
                    created_at,
                    last_used_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn revoke_agent_token(&self, app: &str, name: &str) -> SkociResult<u32> {
        let conn = self.conn.lock();
        let rows = conn
            .execute(
                "DELETE FROM agent_tokens WHERE app = ?1 AND name = ?2",
                params![app, name],
            )
            .map_err(|e| SkociError::internal(format!("Failed to revoke token: {e}"), "Try again"))?;
        Ok(rows as u32)
    }

    // --- Approvals ---

    pub fn insert_approval(&self, rec: &ApprovalRecord) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO approvals (id, app, action, detail, reason, status, requested_by, decided_by, payload_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                rec.id,
                rec.app,
                rec.action,
                rec.detail,
                rec.reason,
                rec.status,
                rec.requested_by,
                rec.decided_by,
                rec.payload_json,
                rec.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| SkociError::internal(format!("Failed to insert approval: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn get_approval(&self, id: &str) -> SkociResult<Option<ApprovalRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, app, action, detail, reason, status, requested_by, decided_by, payload_json, created_at FROM approvals WHERE id = ?1",
            )
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![id], |row| {
                let created_at_str: String = row.get(9)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(ApprovalRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    action: row.get(2)?,
                    detail: row.get(3)?,
                    reason: row.get(4)?,
                    status: row.get(5)?,
                    requested_by: row.get(6)?,
                    decided_by: row.get(7).ok(),
                    payload_json: row.get(8).ok(),
                    created_at,
                })
            })
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn list_pending_approvals(&self, app: Option<&str>) -> SkociResult<Vec<ApprovalRecord>> {
        let conn = self.conn.lock();
        let (sql, params_vec): (String, Vec<rusqlite::types::Value>) = match app {
            Some(a) => (
                "SELECT id, app, action, detail, reason, status, requested_by, decided_by, payload_json, created_at FROM approvals WHERE status = 'pending' AND app = ?1 ORDER BY created_at DESC".to_string(),
                vec![a.to_string().into()],
            ),
            None => (
                "SELECT id, app, action, detail, reason, status, requested_by, decided_by, payload_json, created_at FROM approvals WHERE status = 'pending' ORDER BY created_at DESC".to_string(),
                vec![],
            ),
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(&params_slice[..], |row| {
                let created_at_str: String = row.get(9)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(ApprovalRecord {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    action: row.get(2)?,
                    detail: row.get(3)?,
                    reason: row.get(4)?,
                    status: row.get(5)?,
                    requested_by: row.get(6)?,
                    decided_by: row.get(7).ok(),
                    payload_json: row.get(8).ok(),
                    created_at,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    pub fn update_approval_status(
        &self,
        id: &str,
        status: &str,
        decided_by: Option<&str>,
    ) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE approvals SET status = ?1, decided_by = ?2 WHERE id = ?3",
            params![status, decided_by, id],
        )
        .map_err(|e| SkociError::internal(format!("Failed to update approval status: {e}"), "Try again"))?;
        Ok(())
    }

    // --- Audit Logs ---

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
            r#"
            INSERT INTO audit_logs (timestamp, actor, actor_type, action, target, details)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![now, actor, actor_type, action, target, details],
        )
        .map_err(|e| SkociError::internal(format!("Failed to record audit log: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn list_audits(&self, limit: usize) -> SkociResult<Vec<AuditEntry>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT id, timestamp, actor, actor_type, action, target, details FROM audit_logs ORDER BY id DESC LIMIT {limit}"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let rows = stmt
            .query_map([], |row| {
                let ts_str: String = row.get(1)?;
                let timestamp = DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp,
                    actor: row.get(2)?,
                    actor_type: row.get(3)?,
                    action: row.get(4)?,
                    target: row.get(5)?,
                    details: row.get(6)?,
                })
            })
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| SkociError::internal(e.to_string(), "Try again"))?);
        }
        Ok(list)
    }

    // --- Settings ---

    pub fn get_setting(&self, key: &str) -> SkociResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        let opt = stmt
            .query_row(params![key], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;

        Ok(opt)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO settings (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![key, value],
        )
        .map_err(|e| SkociError::internal(format!("Failed to save setting: {e}"), "Try again"))?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> SkociResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
            .map_err(|e| SkociError::internal(e.to_string(), "Try again"))?;
        Ok(())
    }

    pub fn get_base_domain(&self) -> SkociResult<Option<String>> {
        self.get_setting("base_domain")
    }

    pub fn set_base_domain(&self, domain: &str) -> SkociResult<()> {
        self.set_setting("base_domain", domain)
    }

    // --- Retention Pruning ---

    pub fn prune_deployments(
        &self,
        keep_per_app: usize,
        cutoff: Option<DateTime<Utc>>,
    ) -> SkociResult<usize> {
        let conn = self.conn.lock();
        let cutoff_str = cutoff.map(|c| c.to_rfc3339());
        let keep_n = keep_per_app as i64;

        let sql = r#"
            DELETE FROM deployments
            WHERE id NOT IN (
                -- Referenced by live routes
                SELECT d.id
                FROM deployments d
                JOIN routes r ON d.app = r.app AND (d.version = r.version OR d.container = r.container)
            )
            AND id NOT IN (
                -- Referenced by pending approvals
                SELECT d.id
                FROM deployments d
                JOIN approvals a ON d.app = a.app AND a.status = 'pending'
                WHERE (
                    a.action = 'app_delete'
                    OR a.payload_json LIKE '%' || d.version || '%'
                    OR a.payload_json LIKE '%' || d.id || '%'
                    OR a.detail LIKE '%' || d.version || '%'
                )
            )
            AND (
                -- When cutoff is None: delete everything beyond newest keep_n per app
                (?2 IS NULL AND id NOT IN (
                    SELECT id FROM (
                        SELECT id, ROW_NUMBER() OVER (PARTITION BY app ORDER BY created_at DESC, id DESC) as rn
                        FROM deployments
                    ) WHERE rn <= ?1
                ))
                OR
                -- When cutoff is Some: delete if beyond keep_n OR (older than cutoff and not rn <= 1)
                (?2 IS NOT NULL AND (
                    id NOT IN (
                        SELECT id FROM (
                            SELECT id, ROW_NUMBER() OVER (PARTITION BY app ORDER BY created_at DESC, id DESC) as rn
                            FROM deployments
                        ) WHERE rn <= ?1
                    )
                    OR
                    (created_at < ?2 AND id NOT IN (
                        SELECT id FROM (
                            SELECT id, ROW_NUMBER() OVER (PARTITION BY app ORDER BY created_at DESC, id DESC) as rn
                            FROM deployments
                        ) WHERE rn <= 1
                    ))
                ))
            )
        "#;

        let deleted = conn
            .execute(sql, params![keep_n, cutoff_str])
            .map_err(|e| SkociError::internal(format!("Failed to prune deployments: {e}"), "Try again"))?;

        Ok(deleted)
    }

    pub fn prune_approvals(&self, cutoff: DateTime<Utc>) -> SkociResult<usize> {
        let conn = self.conn.lock();
        let cutoff_str = cutoff.to_rfc3339();

        let sql = "DELETE FROM approvals WHERE status != 'pending' AND created_at < ?1";
        let deleted = conn
            .execute(sql, params![cutoff_str])
            .map_err(|e| SkociError::internal(format!("Failed to prune approvals: {e}"), "Try again"))?;

        Ok(deleted)
    }

    pub fn prune_audit_logs(&self, cutoff: DateTime<Utc>) -> SkociResult<usize> {
        let conn = self.conn.lock();
        let cutoff_str = cutoff.to_rfc3339();

        let sql = "DELETE FROM audit_logs WHERE timestamp < ?1";
        let deleted = conn
            .execute(sql, params![cutoff_str])
            .map_err(|e| SkociError::internal(format!("Failed to prune audit logs: {e}"), "Try again"))?;

        Ok(deleted)
    }

    pub fn prune_retention(&self, policy: &RetentionPolicy) -> SkociResult<RetentionReport> {
        let now = Utc::now();
        let deployments_cutoff = policy
            .deployments_max_age_days
            .map(|d| now - chrono::Duration::days(d as i64));
        let approvals_cutoff = now - chrono::Duration::days(policy.approvals_max_age_days as i64);
        let audit_logs_cutoff = now - chrono::Duration::days(policy.audit_logs_max_age_days as i64);

        let pruned_deployments = self.prune_deployments(policy.deployments_keep_per_app, deployments_cutoff)?;
        let pruned_approvals = self.prune_approvals(approvals_cutoff)?;
        let pruned_audit_logs = self.prune_audit_logs(audit_logs_cutoff)?;

        {
            let conn = self.conn.lock();
            let _ = conn.execute("PRAGMA optimize", []);
        }

        Ok(RetentionReport {
            pruned_deployments,
            pruned_approvals,
            pruned_audit_logs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_deployments_keeps_newest_and_live_routes() {
        let db = Database::open_in_memory().expect("db open");
        let now = Utc::now();

        // App A deployments
        for i in 1..=5 {
            let dep = DeploymentRecord {
                id: format!("dep-a-{i}"),
                app: "app-a".to_string(),
                env: "production".to_string(),
                version: format!("v{i}"),
                kind: "deploy".to_string(),
                status: "superseded".to_string(),
                commit_hash: "hash".to_string(),
                image: format!("skoci/app-a:v{i}"),
                container: format!("c-a-{i}"),
                host_port: Some(8000 + i as u16),
                triggered_by: "agent".to_string(),
                created_at: now - chrono::Duration::days(10 - i),
                finished_at: Some(now - chrono::Duration::days(10 - i)),
                error_json: None,
                evidence_json: None,
            };
            db.insert_deployment(&dep).expect("insert dep");
        }

        // Live route points to v2 (which is older than v4, v5)
        let route = RouteRecord {
            app: "app-a".to_string(),
            env: "production".to_string(),
            container: "c-a-2".to_string(),
            host_port: 8002,
            version: "v2".to_string(),
            hosts: vec!["app-a.local".to_string()],
            deployed_at: now - chrono::Duration::days(8),
        };
        db.upsert_route(&route).expect("upsert route");

        // Keep newest 2 deployments per app
        let pruned = db.prune_deployments(2, None).expect("prune deployments");
        // We had 5 deployments.
        // Newest 2 are v5 and v4.
        // v2 is referenced by the live route.
        // v1 and v3 are pruned (2 pruned).
        assert_eq!(pruned, 2);

        let remaining = db.list_deployments(Some("app-a"), 10).expect("list deployments");
        let remaining_versions: Vec<String> = remaining.into_iter().map(|d| d.version).collect();
        assert_eq!(remaining_versions.len(), 3);
        assert!(remaining_versions.contains(&"v5".to_string()));
        assert!(remaining_versions.contains(&"v4".to_string()));
        assert!(remaining_versions.contains(&"v2".to_string()));
        assert!(!remaining_versions.contains(&"v1".to_string()));
        assert!(!remaining_versions.contains(&"v3".to_string()));
    }

    #[test]
    fn test_retention_approvals_pending_never_pruned_decided_past_limit_pruned() {
        let db = Database::open_in_memory().expect("db open");
        let now = Utc::now();

        // 1. Pending approval from 100 days ago
        let old_pending = ApprovalRecord {
            id: "appr-old-pending".to_string(),
            app: "app-a".to_string(),
            action: "promote".to_string(),
            detail: "Promote v1".to_string(),
            reason: "Testing".to_string(),
            status: "pending".to_string(),
            requested_by: "agent-1".to_string(),
            decided_by: None,
            payload_json: Some(r#"{"version":"v1"}"#.to_string()),
            created_at: now - chrono::Duration::days(100),
        };
        db.insert_approval(&old_pending).expect("insert old pending");

        // 2. Decided approval from 100 days ago (past age limit)
        let old_decided = ApprovalRecord {
            id: "appr-old-decided".to_string(),
            app: "app-a".to_string(),
            action: "promote".to_string(),
            detail: "Promote v0".to_string(),
            reason: "Testing".to_string(),
            status: "approved".to_string(),
            requested_by: "agent-1".to_string(),
            decided_by: Some("admin".to_string()),
            payload_json: Some(r#"{"version":"v0"}"#.to_string()),
            created_at: now - chrono::Duration::days(100),
        };
        db.insert_approval(&old_decided).expect("insert old decided");

        // 3. Decided approval from 5 days ago (within age limit)
        let recent_decided = ApprovalRecord {
            id: "appr-recent-decided".to_string(),
            app: "app-a".to_string(),
            action: "promote".to_string(),
            detail: "Promote v2".to_string(),
            reason: "Testing".to_string(),
            status: "approved".to_string(),
            requested_by: "agent-1".to_string(),
            decided_by: Some("admin".to_string()),
            payload_json: Some(r#"{"version":"v2"}"#.to_string()),
            created_at: now - chrono::Duration::days(5),
        };
        db.insert_approval(&recent_decided).expect("insert recent decided");

        // Prune approvals older than 30 days
        let cutoff = now - chrono::Duration::days(30);
        let pruned = db.prune_approvals(cutoff).expect("prune approvals");
        assert_eq!(pruned, 1);

        // Pending is never pruned
        assert!(db.get_approval("appr-old-pending").expect("get").is_some());
        // Recent decided is not pruned
        assert!(db.get_approval("appr-recent-decided").expect("get").is_some());
        // Old decided IS pruned
        assert!(db.get_approval("appr-old-decided").expect("get").is_none());
    }

    #[test]
    fn test_retention_deployments_protects_pending_approvals() {
        let db = Database::open_in_memory().expect("db open");
        let now = Utc::now();

        for i in 1..=4 {
            let dep = DeploymentRecord {
                id: format!("dep-b-{i}"),
                app: "app-b".to_string(),
                env: "production".to_string(),
                version: format!("v{i}"),
                kind: "deploy".to_string(),
                status: "superseded".to_string(),
                commit_hash: "hash".to_string(),
                image: format!("skoci/app-b:v{i}"),
                container: format!("c-b-{i}"),
                host_port: Some(8100 + i as u16),
                triggered_by: "agent".to_string(),
                created_at: now - chrono::Duration::days(10 - i),
                finished_at: Some(now - chrono::Duration::days(10 - i)),
                error_json: None,
                evidence_json: None,
            };
            db.insert_deployment(&dep).expect("insert dep");
        }

        // Pending approval references v1
        let pending = ApprovalRecord {
            id: "appr-pending-v1".to_string(),
            app: "app-b".to_string(),
            action: "promote".to_string(),
            detail: "Promote v1".to_string(),
            reason: "Promoting historical".to_string(),
            status: "pending".to_string(),
            requested_by: "agent-1".to_string(),
            decided_by: None,
            payload_json: Some(r#"{"version":"v1"}"#.to_string()),
            created_at: now,
        };
        db.insert_approval(&pending).expect("insert pending");

        // Keep newest 1 deployment
        let pruned = db.prune_deployments(1, None).expect("prune deployments");
        // 4 total: newest 1 is v4. v1 is protected by pending approval. v2 and v3 are pruned.
        assert_eq!(pruned, 2);

        let remaining = db.list_deployments(Some("app-b"), 10).expect("list");
        let versions: Vec<String> = remaining.into_iter().map(|d| d.version).collect();
        assert!(versions.contains(&"v4".to_string()));
        assert!(versions.contains(&"v1".to_string()));
        assert!(!versions.contains(&"v2".to_string()));
        assert!(!versions.contains(&"v3".to_string()));
    }

    #[test]
    fn test_retention_audit_logs_pruned_past_cutoff() {
        let db = Database::open_in_memory().expect("db open");
        let now = Utc::now();

        {
            let conn = db.conn.lock();
            let old_ts = (now - chrono::Duration::days(100)).to_rfc3339();
            conn.execute(
                "INSERT INTO audit_logs (timestamp, actor, actor_type, action, target, details) VALUES (?1, 'admin', 'user', 'deploy', 'app', 'old')",
                params![old_ts],
            ).expect("insert old audit");
        }
        db.record_audit("admin", "user", "deploy", "app", "recent").expect("insert recent audit");

        let cutoff = now - chrono::Duration::days(30);
        let pruned = db.prune_audit_logs(cutoff).expect("prune audits");
        assert_eq!(pruned, 1);

        let remaining = db.list_audits(10).expect("list audits");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].details, "recent");
    }
}
