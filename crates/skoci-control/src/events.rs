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

#[derive(Clone)]
pub struct JobRegistry {
    running_jobs: Arc<Mutex<HashMap<(String, String), String>>>,
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
        let key = (app.to_string(), env.to_string());

        if let Some(existing_id) = running.get(&key) {
            return Err(SkociError::conflict(
                format!("Deployment {existing_id} sedang berjalan untuk {app}/{env}"),
                "Tunggu hingga proses deployment selesai sebelum memulai yang baru",
            ));
        }

        running.insert(key, deployment_id.to_string());

        let buffer = Arc::new(JobEventBuffer::new(
            deployment_id.to_string(),
            app.to_string(),
            env.to_string(),
        ));

        let mut events = self.events.lock();
        events.insert(deployment_id.to_string(), buffer.clone());

        Ok(buffer)
    }

    pub fn finish_job(&self, app: &str, env: &str, deployment_id: &str) {
        {
            let mut running = self.running_jobs.lock();
            let key = (app.to_string(), env.to_string());
            if let Some(id) = running.get(&key) {
                if id == deployment_id {
                    running.remove(&key);
                }
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
        for ((a, _), id) in running.iter() {
            if a == app {
                return Some(id.clone());
            }
        }
        None
    }

    pub fn evict_expired(&self) {
        let ttl = Duration::from_secs(600); // 10 minutes
        let mut events = self.events.lock();
        events.retain(|_, buf| !buf.is_expired(ttl));
    }
}
