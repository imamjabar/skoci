use skoci_api::error::{SkociError, SkociResult};
use std::process::Stdio;
use tokio::process::Command;

pub struct BuildOptions {
    pub tag: String,
    pub dockerfile_path: String,
    pub context_path: String,
    pub memory_limit: Option<String>,
}

pub struct ImageBuilder;

impl ImageBuilder {
    pub async fn build(opts: &BuildOptions) -> SkociResult<()> {
        let mut cmd = Command::new("docker");
        cmd.arg("build");

        cmd.arg("-t").arg(&opts.tag);
        cmd.arg("-f").arg(&opts.dockerfile_path);

        if let Some(mem) = &opts.memory_limit {
            cmd.arg("--memory").arg(mem);
        }

        cmd.arg(&opts.context_path);

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| SkociError::docker_error(format!("Gagal mengeksekusi docker build: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            let out = String::from_utf8_lossy(&output.stdout);
            Err(SkociError::bad_request(
                format!("Build image gagal:\n{out}\n{err}"),
                "Periksa Dockerfile dan dependensi kode sumber",
            ))
        }
    }

    pub async fn prune_cache() -> SkociResult<()> {
        let output = Command::new("docker")
            .arg("builder")
            .arg("prune")
            .arg("-f")
            .output()
            .await
            .map_err(|e| SkociError::docker_error(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(SkociError::docker_error("Gagal membersihkan cache build"))
        }
    }
}
