use crate::db::Database;
use rand::RngCore;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::responses::PromoteResponse;

#[derive(Clone)]
pub struct ApprovalService {
    db: Database,
}

impl ApprovalService {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn request_approval(
        &self,
        app: &str,
        action: &str,
        target_env: &str,
        reason: &str,
        requested_by: &str,
    ) -> SkociResult<PromoteResponse> {
        let mut bytes = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut bytes);
        let request_id = format!("req_{}", hex::encode(bytes));

        self.db.create_approval(&request_id, app, action, target_env, reason, requested_by)?;
        self.db.record_audit(
            requested_by,
            "agent",
            "request_approval",
            app,
            &format!("Meminta izin {action} ke {target_env}: {reason}"),
        )?;

        Ok(PromoteResponse {
            request_id,
            app: app.to_string(),
            target_env: target_env.to_string(),
            status: "pending_approval".to_string(),
        })
    }

    pub fn approve(&self, request_id: &str, approved_by: &str) -> SkociResult<bool> {
        let ok = self.db.approve_request(request_id, approved_by)?;
        if ok {
            self.db.record_audit(
                approved_by,
                "human",
                "approve",
                request_id,
                "Menyetujui permohonan aksi",
            )?;
            Ok(true)
        } else {
            Err(SkociError::bad_request(
                format!("Permohonan approval {request_id} tidak ditemukan atau sudah diproses"),
                "Periksa ID approval dengan 'skoci status'",
            ))
        }
    }

    pub fn reject(&self, request_id: &str, rejected_by: &str) -> SkociResult<bool> {
        let ok = self.db.reject_request(request_id, rejected_by)?;
        if ok {
            self.db.record_audit(
                rejected_by,
                "human",
                "reject",
                request_id,
                "Menolak permohonan aksi",
            )?;
            Ok(true)
        } else {
            Err(SkociError::bad_request(
                format!("Permohonan approval {request_id} tidak ditemukan atau sudah diproses"),
                "Periksa ID approval dengan 'skoci status'",
            ))
        }
    }
}
