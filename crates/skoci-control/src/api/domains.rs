use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::is_magic_dns;
use skoci_api::requests::DomainCheckRequest;
use skoci_api::responses::DomainCheckResponse;
use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub fn validate_domain_host(host: &str) -> SkociResult<()> {
    if host.is_empty() {
        return Err(SkociError::bad_request(
            "Hostname cannot be empty",
            "Provide a valid hostname",
        ));
    }
    if host.len() > 253 {
        return Err(SkociError::bad_request(
            "Hostname exceeds maximum length of 253 characters",
            "Provide a shorter hostname",
        ));
    }
    if host.chars().any(|c| c.is_whitespace()) {
        return Err(SkociError::bad_request(
            "Hostname cannot contain whitespace",
            "Remove whitespace from hostname",
        ));
    }
    if host.contains("://") || host.starts_with("//") {
        return Err(SkociError::bad_request(
            "Hostname cannot contain a URL scheme",
            "Provide only the hostname, without 'http://' or 'https://'",
        ));
    }
    if host.contains('/') {
        return Err(SkociError::bad_request(
            "Hostname cannot contain a path",
            "Provide only the hostname without trailing path",
        ));
    }
    if host.contains(':') {
        return Err(SkociError::bad_request(
            "Hostname cannot contain a port",
            "Provide only the hostname without a port",
        ));
    }
    Ok(())
}

pub async fn resolve_host_ips(host: &str) -> Vec<IpAddr> {
    let host_owned = host.to_string();
    let resolve_fut = tokio::task::spawn_blocking(move || {
        match (host_owned.as_str(), 443).to_socket_addrs() {
            Ok(addrs) => addrs.map(|sa| sa.ip()).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        }
    });

    match tokio::time::timeout(Duration::from_secs(5), resolve_fut).await {
        Ok(Ok(ips)) => {
            let mut seen = HashSet::new();
            let mut unique = Vec::new();
            for ip in ips {
                if seen.insert(ip) {
                    unique.push(ip);
                }
            }
            unique
        }
        _ => Vec::new(),
    }
}

pub async fn handle_domain_check(
    State(state): State<AppState>,
    AuthSubject(_subject): AuthSubject,
    Json(body): Json<DomainCheckRequest>,
) -> Response {
    if let Err(e) = validate_domain_host(&body.host) {
        return api_reply::<DomainCheckResponse>(Err(e));
    }

    let host = body.host.to_lowercase();
    let magic = is_magic_dns(&host);
    let resolved = resolve_host_ips(&host).await;
    let resolved_ips: Vec<String> = resolved.iter().map(|ip| ip.to_string()).collect();

    let expected_ips: Vec<String> = match state.config.public_ip {
        Some(ip) => vec![ip.to_string()],
        None => Vec::new(),
    };

    let points_here = match state.config.public_ip {
        Some(expected) => resolved.contains(&expected),
        None => false,
    };

    let record_hint = if points_here {
        None
    } else {
        state.config.public_ip.map(|ip| match ip {
            IpAddr::V4(v4) => format!("A {host} -> {v4}"),
            IpAddr::V6(v6) => format!("AAAA {host} -> {v6}"),
        })
    };

    api_reply(Ok(DomainCheckResponse {
        host,
        expected_ips,
        resolved_ips,
        points_here,
        magic_dns: magic,
        record_hint,
    }))
}
