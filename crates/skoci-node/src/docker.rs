use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub name: String,
    pub image: String,
    pub port_mappings: Vec<(u16, u16)>, // host_port -> container_port
    pub env_vars: HashMap<String, String>,
    pub memory_limit: Option<String>,
    pub cpu_limit: Option<f32>,
    pub network: Option<String>,
    pub volumes: Vec<(String, String)>, // host_vol -> container_path
    pub restart_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub running: bool,
    pub ports: Vec<u16>,
}

pub struct DockerClient;

impl DockerClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn check_daemon() -> SkociResult<bool> {
        let output = Command::new("docker")
            .arg("info")
            .arg("--format")
            .arg("{{.ServerVersion}}")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) => Ok(out.status.success()),
            Err(_) => Ok(false),
        }
    }

    pub async fn run_container(&self, config: &ContainerConfig) -> SkociResult<String> {
        let mut cmd = Command::new("docker");
        cmd.arg("run").arg("-d");

        cmd.arg("--name").arg(&config.name);

        for (host_p, container_p) in &config.port_mappings {
            cmd.arg("-p").arg(format!("{host_p}:{container_p}"));
        }

        for (k, v) in &config.env_vars {
            cmd.arg("-e").arg(format!("{k}={v}"));
        }

        if let Some(mem) = &config.memory_limit {
            cmd.arg("-m").arg(mem);
        }

        if let Some(cpus) = config.cpu_limit {
            cmd.arg(format!("--cpus={cpus}"));
        }

        if let Some(net) = &config.network {
            cmd.arg("--network").arg(net);
        }

        for (src, dst) in &config.volumes {
            cmd.arg("-v").arg(format!("{src}:{dst}"));
        }

        if let Some(restart) = &config.restart_policy {
            cmd.arg("--restart").arg(restart);
        }

        cmd.arg(&config.image);

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker run: {e}")))?;

        if output.status.success() {
            let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(id)
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Docker run error: {err}")))
        }
    }

    pub async fn stop_container(&self, name_or_id: &str) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("stop")
            .arg("-t")
            .arg("10")
            .arg(name_or_id)
            .output()
            .await
            .map_err(|e| SkociError::docker_error(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Gagal stop container: {err}")))
        }
    }

    pub async fn remove_container(&self, name_or_id: &str) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(name_or_id)
            .output()
            .await
            .map_err(|e| SkociError::docker_error(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Gagal rm container: {err}")))
        }
    }

    pub async fn get_logs(&self, name_or_id: &str, tail: usize) -> SkociResult<Vec<String>> {
        let output = Command::new("docker")
            .arg("logs")
            .arg("--tail")
            .arg(tail.to_string())
            .arg(name_or_id)
            .output()
            .await
            .map_err(|e| SkociError::docker_error(e.to_string()))?;

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        Ok(combined.lines().map(|s| s.to_string()).collect())
    }
}
