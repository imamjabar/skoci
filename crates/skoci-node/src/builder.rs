use skoci_api::error::{SkociError, SkociResult};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub tag: String,
    pub dockerfile: PathBuf,
    pub context: PathBuf,
    pub labels: Vec<(String, String)>,
}

pub struct ImageBuilder;

impl ImageBuilder {
    /// `docker build --progress=plain`; every stdout/stderr line is sent to `log` as it arrives.
    /// On failure returns SkociError::build_failed with the last 30 lines in the message.
    pub async fn build(
        opts: &BuildOptions,
        log: UnboundedSender<String>,
    ) -> SkociResult<()> {
        let mut cmd = Command::new("docker");
        cmd.env("DOCKER_BUILDKIT", "1");
        cmd.arg("build");
        cmd.arg("--progress=plain");
        cmd.arg("-t").arg(&opts.tag);
        cmd.arg("-f").arg(&opts.dockerfile);

        for (k, v) in &opts.labels {
            cmd.arg("--label").arg(format!("{k}={v}"));
        }

        cmd.arg(&opts.context);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            SkociError::docker_error(format!("Failed to run docker build: {e}"))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SkociError::docker_error("Failed to pipe docker build stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SkociError::docker_error("Failed to pipe docker build stderr"))?;

        let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let tx1 = line_tx.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx1.send(line).is_err() {
                    break;
                }
            }
        });

        let tx2 = line_tx.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx2.send(line).is_err() {
                    break;
                }
            }
        });

        drop(line_tx);

        let mut last_lines = VecDeque::with_capacity(30);

        while let Some(line) = line_rx.recv().await {
            let _ = log.send(line.clone());
            if last_lines.len() >= 30 {
                last_lines.pop_front();
            }
            last_lines.push_back(line);
        }

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        let status = child.wait().await.map_err(|e| {
            SkociError::docker_error(format!("Failed to wait for docker build process: {e}"))
        })?;

        if status.success() {
            Ok(())
        } else {
            let details = last_lines.into_iter().collect::<Vec<_>>().join("\n");
            Err(SkociError::build_failed(details))
        }
    }
}
