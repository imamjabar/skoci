use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::is_magic_dns;
use skoci_api::requests::{DomainCheckRequest, DomainSetPrimaryRequest};
use skoci_api::responses::{DomainCheckResponse, DomainSetPrimaryResponse};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
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

pub async fn handle_set_primary(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<DomainSetPrimaryRequest>,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<DomainSetPrimaryResponse>(Err(err));
    }

    if let Err(e) = validate_domain_host(&body.host) {
        return api_reply::<DomainSetPrimaryResponse>(Err(e));
    }

    let host = body.host.trim().to_lowercase();
    if host.parse::<IpAddr>().is_ok() {
        return api_reply::<DomainSetPrimaryResponse>(Err(SkociError::bad_request(
            format!("'{host}' is an IP address, not a domain name"),
            "Provide a registered domain name (e.g. example.com)",
        )));
    }

    if is_magic_dns(&host) {
        return api_reply::<DomainSetPrimaryResponse>(Err(SkociError::bad_request(
            format!("'{host}' is a magic-DNS domain (e.g. sslip.io)"),
            "Provide a domain name you own (e.g. example.com) to leave magic DNS behind",
        )));
    }

    let dns_check_skipped = body.force;
    if !body.force {
        let expected_ips: Vec<String> = match state.config.public_ip {
            Some(ip) => vec![ip.to_string()],
            None => Vec::new(),
        };

        let resolved = resolve_host_ips(&host).await;
        let points_here = match state.config.public_ip {
            Some(ip) => resolved.contains(&ip),
            None => false,
        };
        if !points_here {
            let resolved_ips: Vec<String> = resolved.iter().map(|ip| ip.to_string()).collect();
            return api_reply::<DomainSetPrimaryResponse>(Err(SkociError::dns_mismatch(
                &host,
                &expected_ips,
                &resolved_ips,
            )));
        }

        let api_host = format!("skoci.{host}");
        let resolved_api = resolve_host_ips(&api_host).await;
        let api_points_here = match state.config.public_ip {
            Some(ip) => resolved_api.contains(&ip),
            None => false,
        };
        if !api_points_here {
            let resolved_api_ips: Vec<String> =
                resolved_api.iter().map(|ip| ip.to_string()).collect();
            return api_reply::<DomainSetPrimaryResponse>(Err(SkociError::dns_mismatch(
                &api_host,
                &expected_ips,
                &resolved_api_ips,
            )));
        }
    }

    let previous_base_domain = state.base_domain();
    if let Err(e) = state.db.set_setting("base_domain", &host) {
        return api_reply::<DomainSetPrimaryResponse>(Err(e));
    }

    let _ = state.db.record_audit(
        &subject.identifier(),
        subject.actor_type(),
        "set_primary_domain",
        &host,
        "Server primary domain updated",
    );

    let routes = match state.db.list_routes() {
        Ok(r) => r,
        Err(e) => return api_reply::<DomainSetPrimaryResponse>(Err(e)),
    };

    let mut updated_routes = Vec::new();
    for mut r in routes {
        let new_hosts = match state.db.get_app_config(&r.app) {
            Ok(Some(cfg)) => crate::deploy::compute_hosts(&Some(host.clone()), &cfg, &r.env),
            Ok(None) => vec![crate::deploy::auto_host(&r.app, &host, &r.env)],
            Err(e) => return api_reply::<DomainSetPrimaryResponse>(Err(e)),
        };
        r.hosts = new_hosts.clone();
        let target = SocketAddr::from(([127, 0, 0, 1], r.host_port));

        let route = skoci_proxy::router::Route {
            app: r.app.clone(),
            env: r.env.clone(),
            hosts: new_hosts,
            target,
            version: r.version.clone(),
        };

        if let Err(e) = state.router.upsert_route(route).await {
            return api_reply::<DomainSetPrimaryResponse>(Err(e));
        }

        if let Err(e) = state.db.upsert_route(&r) {
            return api_reply::<DomainSetPrimaryResponse>(Err(e));
        }

        updated_routes.push(format!("{}:{}", r.app, r.env));
    }

    let api_domain = format!("skoci.{host}");
    state
        .router
        .set_control(Some(api_domain.clone()), state.config.bind)
        .await;

    let certificate_hosts = state.router.all_dns_hosts().await;

    api_reply(Ok(DomainSetPrimaryResponse {
        base_domain: host,
        api_domain,
        previous_base_domain,
        certificate_hosts,
        updated_routes,
        dns_check_skipped,
    }))
}
