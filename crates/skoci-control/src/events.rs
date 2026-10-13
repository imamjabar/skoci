use chrono::Utc;
use parking_lot::Mutex;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::responses::{DeployEvent, DeployResponse, DeployStage, StageStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub const MAX_EVENTS_BUFFER: usize = 2000;
pub const MAX_LOG_LINE_CHARS: usize = 400;

pub struct JobEventBuffer {
    pub deployment_id: String,
    pub app: String,
    pub env: String,
    seq: AtomicU64,
    tx: broadcast::Sender<DeployEvent>,
    buffer: Mutex<Vec<DeployEvent>>,
    finished_at: Mutex<Option<Instant>>,
}

impl JobEventBuffer {
    pub fn new(deployment_id: String, app: String, env: String) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            deployment_id,
            app,
            env,
            seq: AtomicU64::new(1),
            tx,
            buffer: Mutex::new(Vec::with_capacity(64)),
            finished_at: Mutex::new(None),
        }
    }

    pub fn emit(
        &self,
        stage: DeployStage,
        status: StageStatus,
        message: impl Into<String>,
        progress: u8,
        result: Option<DeployResponse>,
        error: Option<SkociError>,
    ) {
        let mut msg = message.into();
        if status == StageStatus::Log && msg.chars().count() > MAX_LOG_LINE_CHARS {
            msg = msg.chars().take(MAX_LOG_LINE_CHARS).collect();
        }

        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let event = DeployEvent {
            seq,
            ts: Utc::now(),
            stage,
            status,
            message: msg,
            progress,
            result,
            error,
        };

        {
            let mut buf = self.buffer.lock();
            if buf.len() >= MAX_EVENTS_BUFFER {
                buf.remove(0);
            }
            buf.push(event.clone());
        }

        let _ = self.tx.send(event);
    }

    pub fn snapshot(&self) -> Vec<DeployEvent> {
        self.buffer.lock().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DeployEvent> {
        self.tx.subscribe()
    }

    pub fn mark_finished(&self) {
        let mut lock = self.finished_at.lock();
        if lock.is_none() {
            *lock = Some(Instant::now());
        }
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        let lock = self.finished_at.lock();
        match *lock {
            Some(t) => t.elapsed() >= ttl,
            None => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppLock {
    pub id: String,
    pub env: Option<String>,
    pub is_delete: bool,
}

impl AppLock {
    pub fn deployment(id: impl Into<String>, env: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            env: Some(env.into()),
            is_delete: false,
        }
    }

    pub fn deletion(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            env: None,
            is_delete: true,
        }
    }

    pub fn is_delete(&self) -> bool {
        self.is_delete
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

pub struct AppLockGuard {
    running_jobs: Arc<Mutex<HashMap<String, AppLock>>>,
    app: String,
    active: bool,
}

impl AppLockGuard {
    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn release(mut self) {
        self.active = false;
        let mut running = self.running_jobs.lock();
        running.remove(&self.app);
    }
}

impl Drop for AppLockGuard {
    fn drop(&mut self) {
        if self.active {
            let mut running = self.running_jobs.lock();
            running.remove(&self.app);
        }
    }
}

#[derive(Clone)]
pub struct JobRegistry {
    running_jobs: Arc<Mutex<HashMap<String, AppLock>>>,
    events: Arc<Mutex<HashMap<String, Arc<JobEventBuffer>>>>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            running_jobs: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_job(
        &self,
        app: &str,
        env: &str,
        deployment_id: &str,
    ) -> SkociResult<Arc<JobEventBuffer>> {
        let mut running = self.running_jobs.lock();

        if let Some(existing) = running.get(app) {
            let existing_id = &existing.id;
            let msg = if existing.is_delete {
                format!("App '{app}' is currently being deleted")
            } else {
                match &existing.env {
                    Some(e) => format!("Deployment {existing_id} is already running for {app}/{e}"),
                    None => format!("Deployment {existing_id} is already running for {app}"),
                }
            };
            let fix = if existing.is_delete {
                "Wait for the current deletion to finish before starting a new one"
            } else {
                "Wait for the current deployment to finish before starting a new one"
            };
            return Err(SkociError::conflict(msg, fix));
        }

        running.insert(
            app.to_string(),
            AppLock::deployment(deployment_id, env),
        );

        let buffer = Arc::new(JobEventBuffer::new(
            deployment_id.to_string(),
            app.to_string(),
            env.to_string(),
        ));

        let mut events = self.events.lock();
        events.insert(deployment_id.to_string(), buffer.clone());

        Ok(buffer)
    }

    pub fn finish_job(&self, app: &str, _env: &str, deployment_id: &str) {
        {
            let mut running = self.running_jobs.lock();
            if running.get(app).is_some_and(|e| e.id == deployment_id) {
                running.remove(app);
            }
        }

        let events = self.events.lock();
        if let Some(buf) = events.get(deployment_id) {
            buf.mark_finished();
        }
    }

    pub fn get_buffer(&self, deployment_id: &str) -> Option<Arc<JobEventBuffer>> {
        self.events.lock().get(deployment_id).cloned()
    }

    pub fn get_running_deployment(&self, app: &str) -> Option<String> {
        let running = self.running_jobs.lock();
        running.get(app).map(|l| l.id.clone())
    }

    pub fn acquire_delete_guard(&self, app: &str) -> SkociResult<AppLockGuard> {
        let mut running = self.running_jobs.lock();

        if let Some(existing) = running.get(app) {
            let existing_id = &existing.id;
            let msg = if existing.is_delete {
                format!("App '{app}' is already being deleted")
            } else {
                format!("Deployment {existing_id} for '{app}' is still running")
            };
            let fix = if existing.is_delete {
                "Wait for the current deletion to finish before starting a new one".to_string()
            } else {
                format!("Wait until finished ('skoci watch {existing_id}'), then delete again")
            };
            return Err(SkociError::conflict(msg, fix));
        }

        let del_id = format!("del_{}", uuid::Uuid::new_v4().simple());
        running.insert(app.to_string(), AppLock::deletion(del_id));

        Ok(AppLockGuard {
            running_jobs: self.running_jobs.clone(),
            app: app.to_string(),
            active: true,
        })
    }

    pub fn evict_expired(&self) {
        let ttl = Duration::from_secs(600); // 10 minutes
        let mut events = self.events.lock();
        events.retain(|_, buf| !buf.is_expired(ttl));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_registry_exclusion_per_app() {
        let registry = JobRegistry::new();
        let b1 = registry.register_job("app1", "preview", "dep1");
        assert!(b1.is_ok());

        let b2 = registry.register_job("app1", "production", "dep2");
        assert!(b2.is_err());
        let err = b2.err().unwrap();
        assert_eq!(err.code, "conflict");
        assert!(err.message.contains("dep1"));

        // Different app succeeds
        let b3 = registry.register_job("app2", "production", "dep3");
        assert!(b3.is_ok());

        // Finishing b1 releases app1
        registry.finish_job("app1", "preview", "dep1");
        let b4 = registry.register_job("app1", "production", "dep4");
        assert!(b4.is_ok());
    }

    #[test]
    fn test_delete_guard_blocks_concurrent_job_and_releases_on_drop() {
        let registry = JobRegistry::new();
        {
            let guard = registry.acquire_delete_guard("app1");
            assert!(guard.is_ok());

            let res = registry.register_job("app1", "preview", "dep1");
            assert!(res.is_err());
            assert_eq!(res.err().unwrap().code, "conflict");
        }

        // Guard dropped
        let res = registry.register_job("app1", "preview", "dep1");
        assert!(res.is_ok());
    }
}
