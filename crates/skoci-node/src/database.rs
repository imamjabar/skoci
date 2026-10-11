use crate::docker::{ContainerConfig, DockerClient};
use rand::Rng;
use skoci_api::error::SkociResult;
use skoci_api::models::{DatabaseEngine, ServiceConfig};
use std::collections::HashMap;

pub struct DatabaseManager {
    docker: DockerClient,
}

impl DatabaseManager {
    pub fn new() -> Self {
        Self {
            docker: DockerClient::new(),
        }
    }

    pub fn generate_password() -> String {
        let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::thread_rng();
        (0..24)
            .map(|_| {
                let idx = rng.gen_range(0..chars.len());
                chars[idx] as char
            })
            .collect()
    }

    pub async fn provision(
        &self,
        app: &str,
        service_name: &str,
        cfg: &ServiceConfig,
    ) -> SkociResult<(String, String)> {
        let container_name = format!("{app}-{service_name}");
        let volume_name = format!("{app}-{service_name}-data");
        let password = Self::generate_password();

        let (image, env_vars, _port, db_url) = match cfg.engine {
            DatabaseEngine::Postgres => {
                let image = format!("postgres:{}", cfg.version);
                let mut env = HashMap::new();
                env.insert("POSTGRES_DB".to_string(), app.replace('-', "_"));
                env.insert("POSTGRES_USER".to_string(), "skoci".to_string());
                env.insert("POSTGRES_PASSWORD".to_string(), password.clone());

                let url = format!(
                    "postgresql://skoci:{}@{container_name}:5432/{}",
                    password,
                    app.replace('-', "_")
                );
                (image, env, 5432, url)
            }
            DatabaseEngine::Mysql => {
                let image = format!("mysql:{}", cfg.version);
                let mut env = HashMap::new();
                env.insert("MYSQL_DATABASE".to_string(), app.replace('-', "_"));
                env.insert("MYSQL_USER".to_string(), "skoci".to_string());
                env.insert("MYSQL_PASSWORD".to_string(), password.clone());
                env.insert("MYSQL_ROOT_PASSWORD".to_string(), password.clone());

                let url = format!(
                    "mysql://skoci:{}@{container_name}:3306/{}",
                    password,
                    app.replace('-', "_")
                );
                (image, env, 3306, url)
            }
            DatabaseEngine::Valkey => {
                let image = format!("valkey/valkey:{}", cfg.version);
                let env = HashMap::new();
                let url = format!("redis://{container_name}:6379");
                (image, env, 6379, url)
            }
        };

        let container_config = ContainerConfig {
            name: container_name.clone(),
            image,
            port_mappings: vec![], // Internal network only, not published to host
            env_vars,
            memory_limit: cfg.memory.clone().or_else(|| Some("512m".to_string())),
            cpu_limit: Some(1.0),
            network: Some(format!("{app}-net")),
            volumes: vec![(volume_name, "/var/lib/postgresql/data".to_string())],
            restart_policy: Some("unless-stopped".to_string()),
        };

        // If docker is available, run container; otherwise just return url
        if DockerClient::check_daemon().await.unwrap_or(false) {
            let _ = self.docker.run_container(&container_config).await?;
        }

        Ok((container_name, db_url))
    }
}
