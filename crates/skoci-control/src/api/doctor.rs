use axum::extract::State;
use axum::response::Response;
use skoci_api::responses::{DoctorCheck, DoctorResponse};
use skoci_node::docker::DockerClient;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::TlsMode;
use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_doctor(
    State(state): State<AppState>,
    AuthSubject(_subject): AuthSubject,
) -> Response {
    let mut checks = Vec::new();

    // 1. Docker daemon
    let docker_ok = DockerClient::check_daemon().await;
    checks.push(DoctorCheck {
        name: "docker_daemon".to_string(),
        passed: docker_ok,
        message: if docker_ok {
            "Docker daemon is active and responding".to_string()
        } else {
            "Docker daemon is not responding".to_string()
        },
        fix: if !docker_ok {
            Some("Ensure Docker is running: 'sudo systemctl start docker'".to_string())
        } else {
            None
        },
    });

    // 2. Proxy HTTP listening
    let probe_addr = SocketAddr::from(([127, 0, 0, 1], state.config.http_bind.port()));
    let http_ok = matches!(
        timeout(Duration::from_millis(500), TcpStream::connect(probe_addr)).await,
        Ok(Ok(_))
    );
    let bind_errors = state
        .proxy_status
        .as_ref()
        .map(|s| s.bind_errors())
        .unwrap_or_default();
    checks.push(DoctorCheck {
        name: "proxy_http".to_string(),
        passed: http_ok && bind_errors.is_empty(),
        message: if bind_errors.is_empty() && http_ok {
            format!("HTTP proxy listening on {}", state.config.http_bind)
        } else if !bind_errors.is_empty() {
            format!("Proxy failed to bind port: {}", bind_errors.join("; "))
        } else {
            format!("HTTP proxy not responding on port {}", state.config.http_bind.port())
        },
        fix: if http_ok && bind_errors.is_empty() {
            None
        } else {
            Some(format!(
                "Port in use by another process? Check: sudo ss -ltnp 'sport = :{}' (old container: sudo docker ps --filter publish={}), stop it, then 'sudo systemctl restart skoci'",
                state.config.http_bind.port(),
                state.config.http_bind.port()
            ))
        },
    });

    // 3. TLS / certificate state
    match state.config.tls {
        TlsMode::Off => checks.push(DoctorCheck {
            name: "tls".to_string(),
            passed: true,
            message: "TLS disabled (plain HTTP). Access API from outside server via SSH tunnel".to_string(),
            fix: None,
        }),
        TlsMode::Acme => {
            let certs = state
                .proxy_status
                .as_ref()
                .map(|s| s.certificates())
                .unwrap_or_default();
            let pending: Vec<String> = certs
                .iter()
                .filter(|(_, issued, _)| !issued)
                .map(|(host, _, err)| match err {
                    Some(e) => format!("{host} ({e})"),
                    None => format!("{host} (in progress)"),
                })
                .collect();
            let issued = certs.iter().filter(|(_, issued, _)| *issued).count();
            checks.push(DoctorCheck {
                name: "tls".to_string(),
                passed: pending.is_empty(),
                message: if pending.is_empty() {
                    format!("HTTPS certificates active for {issued} host(s)")
                } else {
                    format!("Certificates not active yet: {}", pending.join(", "))
                },
                fix: if pending.is_empty() {
                    None
                } else {
                    Some("Ensure ports 80 and 443 are open in VPS firewall and host DNS points to server IP; Skoci will retry automatically".to_string())
                },
            });
        }
    }

    // 4. Data directory writable
    let test_file = state.config.data_dir.join(".doctor_writable_test");
    let writable = tokio::fs::write(&test_file, b"ok").await.is_ok();
    let _ = tokio::fs::remove_file(&test_file).await;
    checks.push(DoctorCheck {
        name: "data_dir_writable".to_string(),
        passed: writable,
        message: if writable {
            format!("Data directory '{}' is writable", state.config.data_dir.display())
        } else {
            format!(
                "Data directory '{}' is not writable",
                state.config.data_dir.display()
            )
        },
        fix: if !writable {
            Some(format!(
                "Check data directory permissions: sudo chown -R $USER '{}'",
                state.config.data_dir.display()
            ))
        } else {
            None
        },
    });

    // 5. SQLite PRAGMA quick_check
    let db_ok = state.db.quick_check().unwrap_or(false);
    checks.push(DoctorCheck {
        name: "database_integrity".to_string(),
        passed: db_ok,
        message: if db_ok {
            "SQLite database integrity OK".to_string()
        } else {
            "SQLite database failed quick_check".to_string()
        },
        fix: if !db_ok {
            Some("Database corrupt, check skoci.db file".to_string())
        } else {
            None
        },
    });

    // 6. Base domain
    let base_domain_set = state.config.base_domain.is_some();
    checks.push(DoctorCheck {
        name: "base_domain".to_string(),
        passed: true,
        message: match &state.config.base_domain {
            Some(d) => format!("Base domain configured: {d}"),
            None => "Base domain not configured (using specific hosts from skoci.yaml)".to_string(),
        },
        fix: if !base_domain_set {
            Some("Set --base-domain <domain> to enable automatic host routing".to_string())
        } else {
            None
        },
    });

    let ram_mb = get_idle_ram_mb();
    let healthy = checks.iter().all(|c| c.passed);

    api_reply(Ok(DoctorResponse {
        healthy,
        version: env!("CARGO_PKG_VERSION").to_string(),
        idle_ram_mb: ram_mb,
        checks,
    }))
}

fn get_idle_ram_mb() -> f32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
            for line in content.lines() {
                if line.starts_with("VmRSS:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<f32>() {
                            return kb / 1024.0;
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    {
        unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
                #[cfg(target_os = "macos")]
                return (usage.ru_maxrss as f32) / (1024.0 * 1024.0);
                #[cfg(not(target_os = "macos"))]
                return (usage.ru_maxrss as f32) / 1024.0;
            }
        }
    }

    0.0
}
