use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{ActionPermission, EnvPolicy, SkociPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Deploy,
    Rollback,
    DbRestore,
    Delete,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Deploy => "deploy",
            Action::Rollback => "rollback",
            Action::DbRestore => "db_restore",
            Action::Delete => "delete",
        }
    }
}

pub struct PolicyEvaluator;

impl PolicyEvaluator {
    pub fn evaluate(
        policy: &SkociPolicy,
        env: &str,
        action: Action,
        is_human: bool,
    ) -> SkociResult<()> {
        let env_policy: &EnvPolicy = match env {
            "preview" | "staging" | "dev" => &policy.preview,
            "production" | "prod" => &policy.production,
            _ => &policy.production, // Default to strict production policy
        };

        let permission = match action {
            Action::Deploy => env_policy.deploy,
            Action::Rollback => env_policy.rollback,
            Action::DbRestore => env_policy.db_restore,
            Action::Delete => env_policy.delete,
        };

        match permission {
            ActionPermission::Auto => Ok(()),
            ActionPermission::Approve => {
                if is_human {
                    Ok(())
                } else {
                    Err(SkociError::approval_required(
                        "pending",
                        action.as_str(),
                    ))
                }
            }
            ActionPermission::Deny => {
                Err(SkociError::policy_violation(action.as_str(), env))
            }
        }
    }
}
