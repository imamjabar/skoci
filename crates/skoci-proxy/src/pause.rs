use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};

#[derive(Clone)]
pub struct PauseController {
    paused: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Default for PauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseController {
    pub fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub async fn wait_if_paused(&self, max_wait: Duration) -> bool {
        if !self.is_paused() {
            return true;
        }

        let wait_fut = async {
            while self.is_paused() {
                self.notify.notified().await;
            }
        };

        timeout(max_wait, wait_fut).await.is_ok()
    }
}
