use skoci_api::error::{SkociError, SkociResult};
use std::net::SocketAddr;
use std::time::Duration;

pub struct HealthChecker {
    client: reqwest::Client,
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new(Duration::from_secs(3))
    }
}

impl HealthChecker {
    pub fn new(per_request_timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(per_request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { client }
    }

    pub async fn probe(&self, addr: SocketAddr, path: &str) -> SkociResult<u16> {
        let path_clean = if path.is_empty() {
            "/"
        } else if path.starts_with('/') {
            path
        } else {
            &format!("/{path}")
        };
        let url = format!("http://{addr}{path_clean}");
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().as_u16()),
            Err(e) => Err(SkociError::health_check_failed(&url, &e.to_string())),
        }
    }

    pub async fn wait_for_healthy(
        &self,
        addr: SocketAddr,
        path: &str,
        total: Duration,
    ) -> SkociResult<()> {
        let start = tokio::time::Instant::now();
        let mut last_detail;

        loop {
            match self.probe(addr, path).await {
                Ok(status) if (200..400).contains(&status) => return Ok(()),
                Ok(status) => {
                    last_detail = format!("aplikasi membalas HTTP {status}");
                }
                Err(_) => {
                    last_detail = "aplikasi belum menerima koneksi di port tersebut".to_string();
                }
            }

            let remaining = total.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(SkociError::health_check_failed(
                    &format!("http://{addr}{path}"),
                    &format!("{last_detail} setelah {} detik", total.as_secs()),
                ));
            }
            tokio::time::sleep(Duration::from_secs(1).min(remaining)).await;
        }
    }
}
