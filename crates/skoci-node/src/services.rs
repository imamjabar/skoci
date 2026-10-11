use crate::docker::{ContainerConfig, ContainerState, DockerClient};
use rand::Rng;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{DatabaseEngine, ServiceConfig};
use std::fs::OpenOptions;
use std::io::Write;

pub struct ServiceSpec<'a> {
    pub app: &'a str,
    pub service: &'a str,
    pub cfg: &'a ServiceConfig,
    pub password: &'a str,
}

#[derive(Clone, Default)]
pub struct ServiceManager {
    docker: DockerClient,
}

impl ServiceManager {
    pub fn new(docker: DockerClient) -> Self {
        Self { docker }
    }

    pub fn network_name(app: &str) -> String {
        format!("skoci-{app}")
    }

    pub fn container_name(app: &str, service: &str) -> String {
        format!("skoci-{app}-{service}")
    }

    pub fn env_var_name(service: &str) -> String {
        if service == "db" {
            "DATABASE_URL".to_string()
        } else {
            let sanitized: String = service
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
                .collect();
            format!("{sanitized}_URL")
        }
    }

    pub fn password_secret_key(service: &str) -> String {
        let sanitized: String = service
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect();
        format!("SKOCI_{sanitized}_PASSWORD")
    }

    pub fn connection_url(spec: &ServiceSpec) -> String {
        let db_name = spec.app.replace('-', "_");
        match spec.cfg.engine {
            DatabaseEngine::Postgres => {
                format!(
                    "postgresql://skoci:{}@{}:5432/{}",
                    spec.password, spec.service, db_name
                )
            }
            DatabaseEngine::Mysql => {
                format!(
                    "mysql://skoci:{}@{}:3306/{}",
                    spec.password, spec.service, db_name
                )
            }
            DatabaseEngine::Valkey => {
                format!("redis://:{}@{}:6379", spec.password, spec.service)
            }
        }
    }

    pub fn generate_password() -> String {
        const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| {
                let idx = rng.gen_range(0..ALPHANUMERIC.len());
                ALPHANUMERIC[idx] as char
            })
            .collect()
    }

    pub async fn state(&self, app: &str, service: &str) -> SkociResult<ContainerState> {
        let name = Self::container_name(app, service);
        self.docker.inspect(&name).await
    }

    pub async fn remove(&self, app: &str, service: &str) -> SkociResult<()> {
        let name = Self::container_name(app, service);
        let _ = self.docker.stop_container(&name, 10).await;
        self.docker.remove_container(&name).await
    }

    pub async fn ensure(&self, spec: &ServiceSpec<'_>) -> SkociResult<()> {
        let container_name = Self::container_name(spec.app, spec.service);
        let current_state = self.docker.inspect(&container_name).await?;

        if current_state.exists && current_state.running {
            return Ok(());
        }

        if current_state.exists && !current_state.running {
            self.docker.start_container(&container_name).await?;
            return self.wait_ready(spec, &container_name).await;
        }

        let net_name = Self::network_name(spec.app);
        self.docker.ensure_network(&net_name).await?;

        let vol_name = format!("skoci-{}-{}-data", spec.app, spec.service);
        let memory_limit = spec
            .cfg
            .memory
            .clone()
            .or_else(|| Some("512m".to_string()));

        let db_name = spec.app.replace('-', "_");
        let (image, mount_path, env_content, command) = match spec.cfg.engine {
            DatabaseEngine::Postgres => (
                format!("postgres:{}", spec.cfg.version),
                "/var/lib/postgresql/data",
                format!(
                    "POSTGRES_DB={}\nPOSTGRES_USER=skoci\nPOSTGRES_PASSWORD={}\n",
                    db_name, spec.password
                ),
                Vec::new(),
            ),
            DatabaseEngine::Mysql => (
                format!("mysql:{}", spec.cfg.version),
                "/var/lib/mysql",
                format!(
                    "MYSQL_DATABASE={}\nMYSQL_USER=skoci\nMYSQL_PASSWORD={}\nMYSQL_ROOT_PASSWORD={}\n",
                    db_name, spec.password, spec.password
                ),
                Vec::new(),
            ),
            DatabaseEngine::Valkey => (
                format!("valkey/valkey:{}", spec.cfg.version),
                "/data",
                format!("VALKEY_PASSWORD={}\n", spec.password),
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "exec valkey-server --appendonly yes --requirepass \"$VALKEY_PASSWORD\"".to_string(),
                ],
            ),
        };

        let random_suffix: u64 = rand::random();
        let env_file_path = std::env::temp_dir().join(format!("skoci-{:016x}.env", random_suffix));

        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        {
            let mut file = opts.open(&env_file_path).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create env file: {e}"),
                    "Check temp directory permissions",
                )
            })?;
            file.write_all(env_content.as_bytes()).map_err(|e| {
                let _ = std::fs::remove_file(&env_file_path);
                SkociError::internal(
                    format!("Failed to write env file: {e}"),
                    "Check temp disk space",
                )
            })?;
        }

        let config = ContainerConfig {
            name: container_name.clone(),
            image,
            publish: vec![],
            env_file: Some(env_file_path.clone()),
            labels: vec![
                ("skoci.app".to_string(), spec.app.to_string()),
                ("skoci.service".to_string(), spec.service.to_string()),
            ],
            memory_limit,
            cpu_limit: None,
            network: Some(net_name),
            network_aliases: vec![spec.service.to_string()],
            volumes: vec![(vol_name, mount_path.to_string())],
            restart_policy: Some("unless-stopped".to_string()),
            command,
        };

        let run_res = self.docker.run_container(&config).await;
        let _ = tokio::fs::remove_file(&env_file_path).await;
        run_res?;

        self.wait_ready(spec, &container_name).await
    }

    async fn wait_ready(&self, spec: &ServiceSpec<'_>, container_name: &str) -> SkociResult<()> {
        let readiness_argv: &[&str] = match spec.cfg.engine {
            DatabaseEngine::Postgres => &["pg_isready", "-U", "skoci"],
            DatabaseEngine::Mysql => &[
                "sh",
                "-c",
                "mysqladmin ping -h 127.0.0.1 -uroot -p\"$MYSQL_ROOT_PASSWORD\" --silent",
            ],
            DatabaseEngine::Valkey => &["sh", "-c", "valkey-cli -a \"$VALKEY_PASSWORD\" ping"],
        };

        for _ in 0..45 {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            if self.docker.exec(container_name, readiness_argv).await.is_ok() {
                return Ok(());
            }
        }

        let logs = self
            .docker
            .logs(container_name, 20)
            .await
            .unwrap_or_default()
            .join("\n");

        Err(SkociError::docker_error(format!(
            "Database '{}' failed to become ready within 90 seconds. Container logs:\n{}",
            spec.service, logs
        )))
    }
}
