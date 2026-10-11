use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentState {
    Idle,
    Building,
    Deploying,
    HealthChecking,
    SmokeTesting,
    Live,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentContext {
    pub app: String,
    pub version: String,
    pub commit: String,
    pub env: String,
    pub state: DeploymentState,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<SkociError>,
}

impl DeploymentContext {
    pub fn new(app: impl Into<String>, version: impl Into<String>, commit: impl Into<String>, env: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            app: app.into(),
            version: version.into(),
            commit: commit.into(),
            env: env.into(),
            state: DeploymentState::Idle,
            started_at: now,
            updated_at: now,
            error: None,
        }
    }

    pub fn transition_to(&mut self, next: DeploymentState) -> SkociResult<()> {
        let valid = match (self.state, next) {
            (DeploymentState::Idle, DeploymentState::Building) => true,
            (DeploymentState::Idle, DeploymentState::Deploying) => true, // If prebuilt
            (DeploymentState::Building, DeploymentState::Deploying) => true,
            (DeploymentState::Deploying, DeploymentState::HealthChecking) => true,
            (DeploymentState::HealthChecking, DeploymentState::SmokeTesting) => true,
            (DeploymentState::SmokeTesting, DeploymentState::Live) => true,
            // Error transitions from active states
            (DeploymentState::Building, DeploymentState::Failed) => true,
            (DeploymentState::Deploying, DeploymentState::Failed) => true,
            (DeploymentState::HealthChecking, DeploymentState::Failed) => true,
            (DeploymentState::SmokeTesting, DeploymentState::Failed) => true,
            // Rollback transition
            (DeploymentState::Live, DeploymentState::RolledBack) => true,
            (DeploymentState::Failed, DeploymentState::RolledBack) => true,
            _ => false,
        };

        if !valid {
            return Err(SkociError::bad_request(
                format!("Transisi state deploy tidak valid: {:?} -> {:?}", self.state, next),
                "Operasi deployment harus mengikuti urutan Build -> Deploy -> HealthCheck -> SmokeTest -> Live",
            ));
        }

        self.state = next;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn fail(&mut self, err: SkociError) {
        self.state = DeploymentState::Failed;
        self.error = Some(err);
        self.updated_at = Utc::now();
    }
}
