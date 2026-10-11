use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use skoci_api::error::{SkociError, SkociResult};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortPublish {
    pub host_ip: String,
    pub host_port: Option<u16>,
    pub container_port: u16,
}

#[derive(Default, Debug, Clone)]
pub struct ContainerConfig {
    pub name: String,
    pub image: String,
    pub publish: Vec<PortPublish>,
    pub env_file: Option<std::path::PathBuf>,
    pub labels: Vec<(String, String)>,
    pub memory_limit: Option<String>,
    pub cpu_limit: Option<f32>,
    pub network: Option<String>,
    pub network_aliases: Vec<String>,
    pub volumes: Vec<(String, String)>,
    pub restart_policy: Option<String>,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerState {
    pub exists: bool,
    pub running: bool,
    pub status: String,
    pub image: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Default)]
pub struct DockerClient;

impl DockerClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn check_daemon() -> bool {
        let output = Command::new("docker")
            .arg("info")
            .arg("--format")
            .arg("{{.ServerVersion}}")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    pub async fn version(&self) -> SkociResult<String> {
        let output = Command::new("docker")
            .arg("version")
            .arg("--format")
            .arg("{{.Server.Version}}")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal mengecek versi docker: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Docker version error: {err}")))
        }
    }

    pub async fn ensure_network(&self, name: &str) -> SkociResult<()> {
        let inspect = Command::new("docker")
            .arg("network")
            .arg("inspect")
            .arg(name)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal inspect network: {e}")))?;

        if inspect.status.success() {
            return Ok(());
        }

        let create = Command::new("docker")
            .arg("network")
            .arg("create")
            .arg(name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal create network: {e}")))?;

        if create.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&create.stderr);
            if err.contains("already exists") {
                Ok(())
            } else {
                Err(SkociError::docker_error(format!("Gagal membuat network '{name}': {err}")))
            }
        }
    }

    pub async fn run_container(&self, cfg: &ContainerConfig) -> SkociResult<String> {
        let mut cmd = Command::new("docker");
        cmd.arg("run").arg("-d");

        if !cfg.name.is_empty() {
            cmd.arg("--name").arg(&cfg.name);
        }

        for p in &cfg.publish {
            let port_str = match p.host_port {
                Some(hp) => format!("{}:{}:{}", p.host_ip, hp, p.container_port),
                None => format!("{}::{}", p.host_ip, p.container_port),
            };
            cmd.arg("-p").arg(port_str);
        }

        if let Some(env_file) = &cfg.env_file {
            cmd.arg("--env-file").arg(env_file);
        }

        for (k, v) in &cfg.labels {
            cmd.arg("--label").arg(format!("{k}={v}"));
        }

        if let Some(mem) = &cfg.memory_limit {
            cmd.arg("-m").arg(mem);
        }

        if let Some(cpus) = cfg.cpu_limit {
            cmd.arg(format!("--cpus={cpus}"));
        }

        if let Some(net) = &cfg.network {
            cmd.arg("--network").arg(net);
        }

        for alias in &cfg.network_aliases {
            cmd.arg("--network-alias").arg(alias);
        }

        for (src, dst) in &cfg.volumes {
            cmd.arg("-v").arg(format!("{src}:{dst}"));
        }

        if let Some(restart) = &cfg.restart_policy {
            cmd.arg("--restart").arg(restart);
        }

        cmd.arg(&cfg.image);

        for arg in &cfg.command {
            cmd.arg(arg);
        }

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker run: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Docker run error: {err}")))
        }
    }

    pub async fn host_port(&self, container: &str, container_port: u16) -> SkociResult<u16> {
        let output = Command::new("docker")
            .arg("port")
            .arg(container)
            .arg(format!("{container_port}/tcp"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker port: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(SkociError::docker_error(format!("Docker port error: {err}")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let target = if let Some((_, right)) = line.split_once("->") {
                right.trim()
            } else {
                line
            };

            if !target.starts_with('[') {
                if let Some((ip, port_str)) = target.rsplit_once(':') {
                    if ip.contains('.') {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            return Ok(port);
                        }
                    }
                }
            }
        }

        Err(SkociError::docker_error(format!(
            "Tidak dapat menemukan mapping host port IPv4 untuk container '{container}' port '{container_port}'"
        )))
    }

    pub async fn inspect(&self, container: &str) -> SkociResult<ContainerState> {
        let output = Command::new("docker")
            .arg("inspect")
            .arg("--format")
            .arg(r#"{"state": {{json .State}}, "image": {{json .Config.Image}}}"#)
            .arg(container)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker inspect: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err.contains("no such") || err.contains("not found") {
                return Ok(ContainerState {
                    exists: false,
                    running: false,
                    status: "missing".to_string(),
                    image: None,
                    started_at: None,
                });
            }
            return Err(SkociError::docker_error(format!(
                "Docker inspect error: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let val: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| {
            SkociError::docker_error(format!("Format JSON inspect tidak valid: {e}"))
        })?;

        let state = if val.get("state").is_some() {
            &val["state"]
        } else {
            &val
        };

        let running = state.get("Running").and_then(|v| v.as_bool()).unwrap_or(false);
        let status = state
            .get("Status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let image = val
            .get("image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        let started_at = state
            .get("StartedAt")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                if s.starts_with("0001-01-01") {
                    None
                } else {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                }
            });

        Ok(ContainerState {
            exists: true,
            running,
            status,
            image,
            started_at,
        })
    }

    pub async fn start_container(&self, name: &str) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("start")
            .arg(name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal start container: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Gagal start container '{name}': {err}")))
        }
    }

    pub async fn stop_container(&self, name: &str, timeout_secs: u32) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("stop")
            .arg("-t")
            .arg(timeout_secs.to_string())
            .arg(name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal stop container: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Gagal stop container '{name}': {err}")))
        }
    }

    pub async fn remove_container(&self, name: &str) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal remove container: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err.contains("no such") || err.contains("not found") {
                Ok(())
            } else {
                Err(SkociError::docker_error(format!(
                    "Gagal remove container '{name}': {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }
    }

    pub async fn logs(&self, container: &str, tail: usize) -> SkociResult<Vec<String>> {
        let output = Command::new("docker")
            .arg("logs")
            .arg("--tail")
            .arg(tail.to_string())
            .arg(container)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal membaca logs: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(SkociError::docker_error(format!("Docker logs error: {err}")));
        }

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        Ok(combined.lines().map(|s| s.to_string()).collect())
    }

    pub async fn image_exists(&self, image: &str) -> SkociResult<bool> {
        let output = Command::new("docker")
            .arg("image")
            .arg("inspect")
            .arg(image)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal inspect image: {e}")))?;

        if output.status.success() {
            Ok(true)
        } else {
            let err = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err.contains("no such") || err.contains("not found") {
                Ok(false)
            } else {
                Err(SkociError::docker_error(format!(
                    "Gagal inspect image '{image}': {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }
    }

    pub async fn remove_image(&self, image: &str) -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("rmi")
            .arg(image)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal remove image: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err.contains("no such") || err.contains("not found") {
                Ok(())
            } else {
                Err(SkociError::docker_error(format!(
                    "Gagal remove image '{image}': {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }
    }

    pub async fn list_containers(&self, label_filter: &str) -> SkociResult<Vec<String>> {
        let output = Command::new("docker")
            .arg("ps")
            .arg("-a")
            .arg("--filter")
            .arg(format!("label={label_filter}"))
            .arg("--format")
            .arg("{{.Names}}")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal list containers: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(SkociError::docker_error(format!("Docker ps error: {err}")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let names = stdout
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(names)
    }

    pub async fn exec(&self, container: &str, argv: &[&str]) -> SkociResult<String> {
        let output = Command::new("docker")
            .arg("exec")
            .arg(container)
            .args(argv)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal spawn docker exec: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!(
                "Docker exec gagal pada container '{container}': {err}"
            )))
        }
    }

    pub async fn exec_to_file(&self, container: &str, argv: &[&str], dest: &Path) -> SkociResult<u64> {
        let mut child = Command::new("docker")
            .arg("exec")
            .arg(container)
            .args(argv)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SkociError::docker_error(format!("Gagal spawn docker exec: {e}")))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| SkociError::docker_error("Stdout tidak dapat dipipe"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| SkociError::docker_error("Stderr tidak dapat dipipe"))?;

        let copy_task = async {
            let mut file = tokio::fs::File::create(dest)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal membuat file tujuan: {e}")))?;
            tokio::io::copy(&mut stdout, &mut file)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal menyalin output ke file: {e}")))
        };

        let stderr_task = async {
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stderr, &mut buf)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal membaca stderr: {e}")))?;
            Ok::<Vec<u8>, SkociError>(buf)
        };

        let (copy_res, stderr_res) = tokio::join!(copy_task, stderr_task);
        let status = child
            .wait()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menunggu proses docker exec: {e}")))?;

        let stderr_bytes = stderr_res.unwrap_or_default();
        let stderr_msg = String::from_utf8_lossy(&stderr_bytes);

        if !status.success() || copy_res.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            let err_detail = if !stderr_msg.trim().is_empty() {
                stderr_msg.trim().to_string()
            } else if let Err(e) = copy_res {
                e.to_string()
            } else {
                format!("Process exited with status {:?}", status.code())
            };
            return Err(SkociError::docker_error(format!(
                "Docker exec to file gagal: {err_detail}"
            )));
        }

        Ok(copy_res.expect("copy succeeded"))
    }

    pub async fn exec_from_file(&self, container: &str, argv: &[&str], src: &Path) -> SkociResult<()> {
        let mut child = Command::new("docker")
            .arg("exec")
            .arg("-i")
            .arg(container)
            .args(argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SkociError::docker_error(format!("Gagal spawn docker exec -i: {e}")))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| SkociError::docker_error("Stdin tidak dapat dipipe"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| SkociError::docker_error("Stderr tidak dapat dipipe"))?;

        let copy_task = async {
            let mut file = tokio::fs::File::open(src)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal membuka file input: {e}")))?;
            tokio::io::copy(&mut file, &mut stdin)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal menyalurkan input ke stdin: {e}")))?;
            drop(stdin);
            Ok::<(), SkociError>(())
        };

        let stderr_task = async {
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stderr, &mut buf)
                .await
                .map_err(|e| SkociError::docker_error(format!("Gagal membaca stderr: {e}")))?;
            Ok::<Vec<u8>, SkociError>(buf)
        };

        let (copy_res, stderr_res) = tokio::join!(copy_task, stderr_task);
        let status = child
            .wait()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menunggu proses docker exec: {e}")))?;

        let stderr_bytes = stderr_res.unwrap_or_default();
        let stderr_msg = String::from_utf8_lossy(&stderr_bytes);

        if !status.success() || copy_res.is_err() {
            let err_detail = if !stderr_msg.trim().is_empty() {
                stderr_msg.trim().to_string()
            } else if let Err(e) = copy_res {
                e.to_string()
            } else {
                format!("Process exited with status {:?}", status.code())
            };
            return Err(SkociError::docker_error(format!(
                "Docker exec from file gagal: {err_detail}"
            )));
        }

        Ok(())
    }

    pub async fn copy_from(&self, container: &str, path: &str, dest: &Path) -> SkociResult<()> {
        let source = format!("{container}:{path}");
        let output = Command::new("docker")
            .arg("cp")
            .arg(&source)
            .arg(dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker cp: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Docker cp from error: {err}")))
        }
    }

    pub async fn copy_to(&self, src: &Path, container: &str, path: &str) -> SkociResult<()> {
        let dest = format!("{container}:{path}");
        let output = Command::new("docker")
            .arg("cp")
            .arg(src)
            .arg(&dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal menjalankan docker cp: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SkociError::docker_error(format!("Docker cp to error: {err}")))
        }
    }
}
