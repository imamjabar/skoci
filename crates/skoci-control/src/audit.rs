use crate::db::Database;
use skoci_api::error::SkociResult;

#[derive(Clone)]
pub struct AuditLogger {
    db: Database,
}

impl AuditLogger {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn log(
        &self,
        actor: &str,
        actor_type: &str,
        action: &str,
        target: &str,
        details: &str,
    ) -> SkociResult<()> {
        self.db.record_audit(actor, actor_type, action, target, details)
    }
}
