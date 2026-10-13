use chrono::{DateTime, Datelike, Timelike, Utc};
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;

use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct CronExpr {
    pub minutes: HashSet<u32>,
    pub hours: HashSet<u32>,
    pub days_of_month: HashSet<u32>,
    pub months: HashSet<u32>,
    pub days_of_week: HashSet<u32>,
}

impl CronExpr {
    pub fn parse(expr: &str) -> SkociResult<Self> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(SkociError::bad_request(
                format!("Cron expression '{}' is invalid (must have 5 fields)", expr),
                "Use 5-field cron format: minute hour day_of_month month day_of_week",
            ));
        }

        let minutes = parse_field(fields[0], 0, 59)?;
        let hours = parse_field(fields[1], 0, 23)?;
        let days_of_month = parse_field(fields[2], 1, 31)?;
        let months = parse_field(fields[3], 1, 12)?;
        let days_of_week = parse_field(fields[4], 0, 7)?; // 0 or 7 = Sunday

        Ok(Self {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
        })
    }

    pub fn is_due(&self, dt: &DateTime<Utc>) -> bool {
        let min = dt.minute();
        let hour = dt.hour();
        let day = dt.day();
        let month = dt.month();
        let mut dow = dt.weekday().num_days_from_sunday();
        if dow == 7 {
            dow = 0;
        }

        self.minutes.contains(&min)
            && self.hours.contains(&hour)
            && self.days_of_month.contains(&day)
            && self.months.contains(&month)
            && (self.days_of_week.contains(&dow) || (dow == 0 && self.days_of_week.contains(&7)))
    }
}

fn parse_field(raw: &str, min: u32, max: u32) -> SkociResult<HashSet<u32>> {
    let mut set = HashSet::new();

    for part in raw.split(',') {
        let part = part.trim();
        if part == "*" {
            for v in min..=max {
                set.insert(v);
            }
        } else if let Some(step_str) = part.strip_prefix("*/") {
            let step: u32 = step_str.parse().map_err(|_| {
                SkociError::bad_request(
                    format!("Cron step value '{}' is invalid", part),
                    "Check cron expression",
                )
            })?;
            if step == 0 {
                return Err(SkociError::bad_request("Cron step cannot be 0", "Use step >= 1"));
            }
            let mut v = min;
            while v <= max {
                set.insert(v);
                v += step;
            }
        } else if let Some((start_str, end_str)) = part.split_once('-') {
            let start: u32 = start_str.parse().map_err(|_| {
                SkociError::bad_request(format!("Cron range '{}' is invalid", part), "Check cron expression")
            })?;
            let end: u32 = end_str.parse().map_err(|_| {
                SkociError::bad_request(format!("Cron range '{}' is invalid", part), "Check cron expression")
            })?;
            if start > end || start < min || end > max {
                return Err(SkociError::bad_request(
                    format!("Cron range '{part}' out of bounds {min}..={max}"),
                    "Check cron expression",
                ));
            }
            for v in start..=end {
                set.insert(v);
            }
        } else {
            let val: u32 = part.parse().map_err(|_| {
                SkociError::bad_request(format!("Cron value '{}' is invalid", part), "Check cron expression")
            })?;
            if val < min || val > max {
                return Err(SkociError::bad_request(
                    format!("Cron value '{val}' out of bounds {min}..={max}"),
                    "Check cron expression",
                ));
            }
            set.insert(val);
        }
    }

    Ok(set)
}

pub async fn run_retention_maintenance(state: &AppState) {
    let policy = crate::db::RetentionPolicy {
        deployments_keep_per_app: state.config.retention_deployments_per_app as usize,
        deployments_max_age_days: Some(state.config.retention_deployments_max_age_days),
        approvals_max_age_days: state.config.retention_approvals_max_age_days,
        audit_logs_max_age_days: state.config.retention_audit_logs_max_age_days,
    };

    match state.db.prune_retention(&policy) {
        Ok(report) => {
            if report.pruned_deployments > 0
                || report.pruned_approvals > 0
                || report.pruned_audit_logs > 0
            {
                tracing::info!(
                    deployments = report.pruned_deployments,
                    approvals = report.pruned_approvals,
                    audit_logs = report.pruned_audit_logs,
                    "Database retention pruning completed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e.message, "Database retention pruning failed");
        }
    }

    // Safe buildx builder cache prune (keep 5 GB of build cache)
    // Does not touch active tagged images or running containers.
    if let Err(e) = state.docker.prune_builder_cache(Some(5 * 1024 * 1024 * 1024)).await {
        tracing::debug!(error = %e.message, "Builder cache prune skipped or failed");
    }
}

pub async fn start_backup_scheduler(state: AppState) {
    tracing::info!("Backup scheduler active");
    let mut last_minute: Option<u32> = None;
    let mut last_retention_hour: Option<u32> = None;

    loop {
        let now = Utc::now();
        let current_minute = now.minute();
        let current_hour = now.hour();

        // Run retention maintenance on startup, then hourly
        if last_retention_hour != Some(current_hour) {
            last_retention_hour = Some(current_hour);
            let state_clone = state.clone();
            tokio::spawn(async move {
                run_retention_maintenance(&state_clone).await;
            });
        }

        // Only run once per minute
        if last_minute != Some(current_minute) {
            last_minute = Some(current_minute);

            if let Ok(configs) = state.db.list_app_configs() {
                for (app, cfg) in configs {
                    for (svc_name, svc_cfg) in cfg.services {
                        if let Some(backup_policy) = &svc_cfg.backup {
                            if let Ok(cron) = CronExpr::parse(&backup_policy.schedule) {
                                if cron.is_due(&now) {
                                    tracing::info!(
                                        app = %app,
                                        service = %svc_name,
                                        "Running scheduled database backup"
                                    );
                                    let bm = state.backup_manager.clone();
                                    let app_clone = app.clone();
                                    let svc_clone = svc_name.clone();
                                    let engine = svc_cfg.engine;
                                    let keep = backup_policy.keep;

                                    tokio::spawn(async move {
                                        match bm.create_backup(&app_clone, &svc_clone, engine).await {
                                            Ok(resp) => {
                                                tracing::info!(
                                                    backup_id = %resp.backup_id,
                                                    app = %app_clone,
                                                    service = %svc_clone,
                                                    "Scheduled backup succeeded"
                                                );
                                                let _ = bm.prune(&app_clone, &svc_clone, keep);
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    app = %app_clone,
                                                    service = %svc_clone,
                                                    error = %e.message,
                                                    "Scheduled backup failed"
                                                );
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        sleep(Duration::from_secs(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_expr_parse_and_is_due() {
        let cron = CronExpr::parse("0 * * * *").unwrap();
        assert!(cron.minutes.contains(&0));
        assert!(!cron.minutes.contains(&1));
    }
}
