use skoci_api::models::{ActionPermission, EnvPolicy, SkociPolicy, ENV_PREVIEW};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    NeedsApproval,
    Deny,
}

pub struct PolicyEvaluator;

impl PolicyEvaluator {
    pub fn evaluate(
        policy: &SkociPolicy,
        env: &str,
        action: Action,
        is_human: bool,
    ) -> Decision {
        let env_policy: &EnvPolicy = if env == ENV_PREVIEW || env == "preview" {
            &policy.preview
        } else {
            &policy.production
        };

        let permission = match action {
            Action::Deploy => env_policy.deploy,
            Action::Rollback => env_policy.rollback,
            Action::DbRestore => env_policy.db_restore,
            Action::Delete => env_policy.delete,
        };

        if is_human {
            match permission {
                ActionPermission::Deny => Decision::Deny,
                _ => Decision::Allow,
            }
        } else {
            match permission {
                ActionPermission::Auto => Decision::Allow,
                ActionPermission::Approve => Decision::NeedsApproval,
                ActionPermission::Deny => Decision::Deny,
            }
        }
    }
}
