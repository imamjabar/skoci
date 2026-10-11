use crate::health::HealthChecker;
use crate::pause::PauseController;
use crate::service::{Service, Target};
use skoci_api::error::{SkociError, SkociResult};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ProxyRouter {
    services: Arc<RwLock<HashMap<String, Service>>>,
    domain_map: Arc<RwLock<HashMap<String, String>>>, // domain -> service_name
    pause_controllers: Arc<RwLock<HashMap<String, PauseController>>>,
    health_checker: Arc<HealthChecker>,
}

impl Default for ProxyRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyRouter {
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            domain_map: Arc::new(RwLock::new(HashMap::new())),
            pause_controllers: Arc::new(RwLock::new(HashMap::new())),
            health_checker: Arc::new(HealthChecker::default()),
        }
    }

    pub async fn register_service(&self, service: Service) {
        let name = service.name.clone();
        let domains = service.domains.clone();

        let mut services = self.services.write().await;
        let mut domain_map = self.domain_map.write().await;
        let mut pause_controllers = self.pause_controllers.write().await;

        for domain in domains {
            domain_map.insert(domain, name.clone());
        }

        pause_controllers
            .entry(name.clone())
            .or_insert_with(PauseController::new);
        services.insert(name, service);
    }

    pub async fn resolve_host(&self, host: &str) -> Option<String> {
        // Strip optional port from host header (e.g. "example.com:80" -> "example.com")
        let clean_host = host.split(':').next().unwrap_or(host);
        let domain_map = self.domain_map.read().await;
        domain_map.get(clean_host).cloned()
    }

    pub async fn get_target_for_request(
        &self,
        service_name: &str,
    ) -> Result<Option<SocketAddr>, SkociError> {
        // Check pause controller
        let pause = {
            let controllers = self.pause_controllers.read().await;
            controllers.get(service_name).cloned()
        };

        if let Some(pause) = pause {
            if !pause.wait_if_paused(Duration::from_secs(5)).await {
                return Err(SkociError::bad_request(
                    "Timeout menunggu deploy cutover selesai",
                    "Coba beberapa detik lagi",
                ));
            }
        }

        let services = self.services.read().await;
        if let Some(service) = services.get(service_name) {
            let rand_val = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() % 100) as u8;
            Ok(service.select_target(rand_val))
        } else {
            Err(SkociError::app_not_found(service_name))
        }
    }

    pub async fn cutover(
        &self,
        service_name: &str,
        new_target: Target,
        health_check: bool,
    ) -> SkociResult<Option<Target>> {
        // 1. Optional health check before starting cutover
        if health_check {
            let path = {
                let services = self.services.read().await;
                services
                    .get(service_name)
                    .map(|s| s.health_path.clone())
                    .unwrap_or_else(|| "/healthz".to_string())
            };
            self.health_checker
                .wait_for_healthy(new_target.addr, &path, 5, Duration::from_millis(500))
                .await?;
        }

        // 2. Pause incoming requests briefly
        let pause = {
            let mut controllers = self.pause_controllers.write().await;
            controllers
                .entry(service_name.to_string())
                .or_insert_with(PauseController::new)
                .clone()
        };

        pause.pause();

        // 3. Swap targets atomically
        let previous = {
            let mut services = self.services.write().await;
            if let Some(service) = services.get_mut(service_name) {
                service.set_candidate(new_target);
                service.promote_candidate()
            } else {
                pause.resume();
                return Err(SkociError::app_not_found(service_name));
            }
        };

        // 4. Resume incoming requests
        pause.resume();

        Ok(previous)
    }

    pub async fn set_maintenance(&self, service_name: &str, enabled: bool) -> SkociResult<()> {
        let mut services = self.services.write().await;
        if let Some(service) = services.get_mut(service_name) {
            service.set_maintenance(enabled);
            Ok(())
        } else {
            Err(SkociError::app_not_found(service_name))
        }
    }

    pub async fn set_rollout_split(&self, service_name: &str, percentage: u8) -> SkociResult<()> {
        let mut services = self.services.write().await;
        if let Some(service) = services.get_mut(service_name) {
            service.set_rollout_split(percentage);
            Ok(())
        } else {
            Err(SkociError::app_not_found(service_name))
        }
    }
}
