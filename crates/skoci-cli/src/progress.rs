use skoci_api::responses::DeployStage;
use std::time::{Duration, Instant};

pub fn format_duration(dur: Duration) -> String {
    let secs = dur.as_secs();
    let mins = secs / 60;
    let rem = secs % 60;
    format!("{mins}:{rem:02}")
}

pub fn format_eta(elapsed_secs: u64, estimated_secs: Option<u64>) -> Option<String> {
    let est = estimated_secs?;
    if elapsed_secs >= est {
        return None;
    }
    let remaining = est.saturating_sub(elapsed_secs);
    if remaining == 0 {
        return None;
    }
    let mins = remaining / 60;
    let secs = remaining % 60;
    Some(format!("~{mins}:{secs:02} left"))
}

pub fn render_progress_bar(
    progress: u8,
    elapsed: Duration,
    estimated_secs: Option<u64>,
    width: usize,
) -> String {
    let pct = progress.min(100);
    let filled = ((pct as usize) * width) / 100;
    let empty = width.saturating_sub(filled);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    let elapsed_str = format_duration(elapsed);
    if let Some(eta) = format_eta(elapsed.as_secs(), estimated_secs) {
        format!("[{bar}] {pct:>3}%  {elapsed_str} ({eta})")
    } else {
        format!("[{bar}] {pct:>3}%  {elapsed_str}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Running,
    Done(Duration),
    Skipped,
    Failed(String),
}

pub fn render_stage_line(
    step_num: usize,
    stage: DeployStage,
    state: &StepState,
    is_term: bool,
) -> String {
    let label = stage.label();
    match state {
        StepState::Pending => {
            if is_term {
                format!("  \x1b[2m○ [{step_num}/8] {label}\x1b[0m")
            } else {
                format!("  ○ [{step_num}/8] {label}")
            }
        }
        StepState::Running => {
            if is_term {
                format!("  \x1b[36m▸ [{step_num}/8] {label}\x1b[0m")
            } else {
                format!("  ▸ [{step_num}/8] {label}")
            }
        }
        StepState::Done(dur) => {
            let secs = dur.as_secs_f64();
            if is_term {
                format!("  \x1b[32m✓ [{step_num}/8] {label} ({secs:.1}s)\x1b[0m")
            } else {
                format!("  ✓ [{step_num}/8] {label} ({secs:.1}s)")
            }
        }
        StepState::Skipped => {
            if is_term {
                format!("  \x1b[33m– [{step_num}/8] {label} (skipped)\x1b[0m")
            } else {
                format!("  – [{step_num}/8] {label} (skipped)")
            }
        }
        StepState::Failed(msg) => {
            let detail = if msg.is_empty() {
                String::new()
            } else {
                format!(": {msg}")
            };
            if is_term {
                format!("  \x1b[31m✗ [{step_num}/8] {label}{detail}\x1b[0m")
            } else {
                format!("  ✗ [{step_num}/8] {label}{detail}")
            }
        }
    }
}

pub fn stage_hint(stage: DeployStage) -> Option<&'static str> {
    match stage {
        DeployStage::Build => Some("Building the Docker image (usually 30-120s on first deploy)"),
        DeployStage::Health => Some("Waiting for container health check to pass"),
        DeployStage::Smoke => Some("Running smoke test against application"),
        _ => None,
    }
}

pub struct StageEntry {
    pub stage: DeployStage,
    pub state: StepState,
    pub started_at: Option<Instant>,
}

pub struct DeployProgressTracker {
    pub stages: Vec<StageEntry>,
    pub current_progress: u8,
    pub start_time: Instant,
    pub estimated_secs: Option<u64>,
    pub min_remaining: Option<u64>,
    pub log_window: Vec<String>,
    pub active_stage_idx: Option<usize>,
}

impl DeployProgressTracker {
    pub fn new(estimated_secs: Option<u64>) -> Self {
        let stages = DeployStage::STEPS
            .into_iter()
            .map(|stage| StageEntry {
                stage,
                state: StepState::Pending,
                started_at: None,
            })
            .collect();

        Self {
            stages,
            current_progress: 0,
            start_time: Instant::now(),
            estimated_secs,
            min_remaining: None,
            log_window: Vec::new(),
            active_stage_idx: None,
        }
    }

    pub fn update_progress(&mut self, progress: u8) {
        self.current_progress = progress.min(100);
    }

    pub fn find_stage_index(&self, stage: DeployStage) -> Option<usize> {
        self.stages.iter().position(|s| s.stage == stage)
    }

    pub fn on_stage_running(&mut self, stage: DeployStage) -> Option<usize> {
        if let Some(idx) = self.find_stage_index(stage) {
            self.stages[idx].state = StepState::Running;
            if self.stages[idx].started_at.is_none() {
                self.stages[idx].started_at = Some(Instant::now());
            }
            self.active_stage_idx = Some(idx);
            self.log_window.clear();
            Some(idx + 1)
        } else {
            None
        }
    }

    pub fn on_stage_ok(&mut self, stage: DeployStage) -> Option<(usize, Duration)> {
        if let Some(idx) = self.find_stage_index(stage) {
            let dur = self.stages[idx]
                .started_at
                .map(|t| t.elapsed())
                .unwrap_or_default();
            self.stages[idx].state = StepState::Done(dur);
            if self.active_stage_idx == Some(idx) {
                self.active_stage_idx = None;
            }
            self.log_window.clear();
            Some((idx + 1, dur))
        } else {
            None
        }
    }

    pub fn on_stage_skipped(&mut self, stage: DeployStage) -> Option<usize> {
        if let Some(idx) = self.find_stage_index(stage) {
            self.stages[idx].state = StepState::Skipped;
            if self.active_stage_idx == Some(idx) {
                self.active_stage_idx = None;
            }
            self.log_window.clear();
            Some(idx + 1)
        } else {
            None
        }
    }

    pub fn on_stage_failed(&mut self, stage: DeployStage, msg: String) -> Option<usize> {
        if let Some(idx) = self.find_stage_index(stage) {
            self.stages[idx].state = StepState::Failed(msg);
            if self.active_stage_idx == Some(idx) {
                self.active_stage_idx = None;
            }
            self.log_window.clear();
            Some(idx + 1)
        } else {
            None
        }
    }

    pub fn on_log(&mut self, msg: String) {
        let truncated = if msg.chars().count() > 100 {
            let mut s: String = msg.chars().take(97).collect();
            s.push_str("...");
            s
        } else {
            msg
        };
        self.log_window.push(truncated);
        if self.log_window.len() > 5 {
            self.log_window.remove(0);
        }
    }

    fn compute_clamped_eta(&mut self) -> Option<String> {
        let est = self.estimated_secs?;
        let elapsed_secs = self.start_time.elapsed().as_secs();
        if elapsed_secs >= est {
            return None;
        }
        let raw_remaining = est.saturating_sub(elapsed_secs);
        if raw_remaining == 0 {
            return None;
        }
        let clamped = match self.min_remaining {
            Some(prev) => raw_remaining.min(prev),
            None => raw_remaining,
        };
        self.min_remaining = Some(clamped);
        let mins = clamped / 60;
        let secs = clamped % 60;
        Some(format!("~{mins}:{secs:02} left"))
    }

    pub fn render_full(&mut self, is_term: bool) -> Vec<String> {
        let mut lines = Vec::new();
        let elapsed = self.start_time.elapsed();
        let pct = self.current_progress;
        let width = 20;
        let filled = ((pct as usize) * width) / 100;
        let empty = width.saturating_sub(filled);
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
        let elapsed_str = format_duration(elapsed);

        if let Some(eta) = self.compute_clamped_eta() {
            lines.push(format!("  [{bar}] {pct:>3}%  {elapsed_str} ({eta})"));
        } else {
            lines.push(format!("  [{bar}] {pct:>3}%  {elapsed_str}"));
        }

        for (i, entry) in self.stages.iter().enumerate() {
            let step_num = i + 1;
            lines.push(render_stage_line(step_num, entry.stage, &entry.state, is_term));
            if self.active_stage_idx == Some(i) {
                if let Some(hint) = stage_hint(entry.stage) {
                    if is_term {
                        lines.push(format!("    \x1b[2m{hint}\x1b[0m"));
                    } else {
                        lines.push(format!("    {hint}"));
                    }
                }
                for log in &self.log_window {
                    if is_term {
                        lines.push(format!("    \x1b[2m│ {log}\x1b[0m"));
                    } else {
                        lines.push(format!("    │ {log}"));
                    }
                }
            }
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_progress_bar_0_50_100_with_and_without_eta() {
        let bar_0 = render_progress_bar(0, Duration::from_secs(0), None, 20);
        assert!(bar_0.contains("  0%"));
        assert!(bar_0.contains("[░░░░░░░░░░░░░░░░░░░░]"));
        assert!(!bar_0.contains("left"));

        let bar_50_eta = render_progress_bar(50, Duration::from_secs(10), Some(20), 20);
        assert!(bar_50_eta.contains(" 50%"));
        assert!(bar_50_eta.contains("[██████████░░░░░░░░░░]"));
        assert!(bar_50_eta.contains("~0:10 left"));

        let bar_100 = render_progress_bar(100, Duration::from_secs(30), None, 20);
        assert!(bar_100.contains("100%"));
        assert!(bar_100.contains("[████████████████████]"));
        assert!(!bar_100.contains("left"));

        let bar_100_with_eta_past = render_progress_bar(100, Duration::from_secs(30), Some(20), 20);
        assert!(bar_100_with_eta_past.contains("100%"));
        assert!(!bar_100_with_eta_past.contains("left"));
    }

    #[test]
    fn test_format_eta_never_negative() {
        assert_eq!(format_eta(10, Some(30)), Some("~0:20 left".to_string()));
        assert_eq!(format_eta(65, Some(125)), Some("~1:00 left".to_string()));
        assert_eq!(format_eta(30, Some(30)), None);
        assert_eq!(format_eta(35, Some(30)), None);
        assert_eq!(format_eta(100, None), None);
    }

    #[test]
    fn test_stage_list_rendering_markers() {
        let pending = render_stage_line(1, DeployStage::Extract, &StepState::Pending, false);
        assert!(pending.contains("○ [1/8] Extract code"));

        let running = render_stage_line(2, DeployStage::Build, &StepState::Running, false);
        assert!(running.contains("▸ [2/8] Build image"));

        let done = render_stage_line(
            3,
            DeployStage::Services,
            &StepState::Done(Duration::from_millis(1500)),
            false,
        );
        assert!(done.contains("✓ [3/8] Prepare services (1.5s)"));

        let skipped = render_stage_line(4, DeployStage::Start, &StepState::Skipped, false);
        assert!(skipped.contains("– [4/8] Start container (skipped)"));

        let failed = render_stage_line(
            5,
            DeployStage::Health,
            &StepState::Failed("timeout".to_string()),
            false,
        );
        assert!(failed.contains("✗ [5/8] Health check: timeout"));
    }
}
