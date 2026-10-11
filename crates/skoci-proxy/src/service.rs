use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub addr: SocketAddr,
    pub version: String,
    pub healthy: bool,
    pub last_checked: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub domains: Vec<String>,
    pub active_target: Option<Target>,
    pub candidate_target: Option<Target>,
    pub maintenance: bool,
    pub health_path: String,
    pub rollout_percentage: u8,
}

impl Service {
    pub fn new(name: impl Into<String>, domains: Vec<String>, health_path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            domains,
            active_target: None,
            candidate_target: None,
            maintenance: false,
            health_path: health_path.into(),
            rollout_percentage: 0,
        }
    }

    pub fn set_maintenance(&mut self, enabled: bool) {
        self.maintenance = enabled;
    }

    pub fn set_candidate(&mut self, target: Target) {
        self.candidate_target = Some(target);
    }

    pub fn promote_candidate(&mut self) -> Option<Target> {
        if let Some(candidate) = self.candidate_target.take() {
            let previous = self.active_target.replace(candidate);
            previous
        } else {
            None
        }
    }

    pub fn rollback_to(&mut self, previous: Target) {
        self.active_target = Some(previous);
        self.candidate_target = None;
    }

    pub fn set_rollout_split(&mut self, percentage: u8) {
        self.rollout_percentage = percentage.min(100);
    }

    pub fn select_target(&self, rand_percent: u8) -> Option<SocketAddr> {
        if self.maintenance {
            return None;
        }
        if self.rollout_percentage > 0 && rand_percent < self.rollout_percentage {
            if let Some(candidate) = &self.candidate_target {
                return Some(candidate.addr);
            }
        }
        self.active_target.as_ref().map(|t| t.addr)
    }
}
