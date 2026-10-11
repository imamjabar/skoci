use skoci_api::error::{SkociError, SkociResult};
use std::net::SocketAddr;
use std::time::Duration;

pub struct HealthChecker {
    client: reqwest::Client,
    _timeout: Duration,
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new(Duration::from_secs(3))
    }
}

impl HealthChecker {
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();
        Self { client, _timeout: timeout }
    }

    pub async fn check(&self, addr: SocketAddr, path: &str) -> SkociResult<()> {
        let url = format!("http://{addr}{path}");
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    Ok(())
                } else {
                    Err(SkociError::health_check_failed(
                        &url,
                        &format!("Status HTTP {}", resp.status()),
                    ))
                }
            }
            Err(e) => Err(SkociError::health_check_failed(&url, &e.to_string())),
        }
    }

    pub async fn wait_for_healthy(
        &self,
        addr: SocketAddr,
        path: &str,
        retries: usize,
        delay: Duration,
    ) -> SkociResult<()> {
        let mut last_err = None;
        for _ in 0..retries {
            match self.check(addr, path).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            SkociError::health_check_failed(
                &addr.to_string(),
                "Timeout menunggu container siap",
            )
        }))
    }
}
