use skoci_proxy::health::HealthChecker;
use skoci_proxy::router::{ProxyRouter, Resolution, Route};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn test_resolution_precedence() {
    let router = ProxyRouter::new();

    let target_app1: SocketAddr = "127.0.0.1:3001".parse().unwrap();
    let target_app2_prev: SocketAddr = "127.0.0.1:3002".parse().unwrap();
    let target_app2_prod: SocketAddr = "127.0.0.1:3003".parse().unwrap();
    let target_control: SocketAddr = "127.0.0.1:8787".parse().unwrap();
    let target_override: SocketAddr = "127.0.0.1:3099".parse().unwrap();

    router
        .upsert_route(Route {
            app: "app1".to_string(),
            env: "production".to_string(),
            hosts: vec!["explicit.com".to_string(), "shared.org".to_string()],
            target: target_app1,
            version: "v1".to_string(),
        })
        .await
        .unwrap();

    router
        .upsert_route(Route {
            app: "app2".to_string(),
            env: "preview".to_string(),
            hosts: vec!["preview.app2.org".to_string()],
            target: target_app2_prev,
            version: "v1".to_string(),
        })
        .await
        .unwrap();

    router
        .upsert_route(Route {
            app: "app2".to_string(),
            env: "production".to_string(),
            hosts: vec!["app2.org".to_string()],
            target: target_app2_prod,
            version: "v1".to_string(),
        })
        .await
        .unwrap();
    router
        .set_control(Some("control.skoci.io".to_string()), target_control)
        .await;

    // 1. Explicit host match
    assert_eq!(
        router.resolve("explicit.com").await,
        Resolution::App {
            key: "app1:production".to_string(),
            target: target_app1
        }
    );

    // Case-insensitivity
    assert_eq!(
        router.resolve("EXPLICIT.COM").await,
        Resolution::App {
            key: "app1:production".to_string(),
            target: target_app1
        }
    );

    // Port stripping
    assert_eq!(
        router.resolve("explicit.com:8080").await,
        Resolution::App {
            key: "app1:production".to_string(),
            target: target_app1
        }
    );
    assert_eq!(
        router.resolve("EXPLICIT.COM:443").await,
        Resolution::App {
            key: "app1:production".to_string(),
            target: target_app1
        }
    );

    // 2. Control host cannot be overridden by app route
    let override_err = router
        .upsert_route(Route {
            app: "override".to_string(),
            env: "production".to_string(),
            hosts: vec!["control.skoci.io".to_string()],
            target: target_override,
            version: "v1".to_string(),
        })
        .await;
    assert!(override_err.is_err());
    assert_eq!(override_err.unwrap_err().code, "conflict");

    // Control still resolves to control
    assert_eq!(
        router.resolve("control.skoci.io").await,
        Resolution::Control(target_control)
    );
    // 3. Control match
    assert_eq!(
        router.resolve("control.skoci.io").await,
        Resolution::Control(target_control)
    );
    assert_eq!(
        router.resolve("CONTROL.SKOCI.IO:8787").await,
        Resolution::Control(target_control)
    );

    // 4. IP default production
    router.set_default_app(Some("app2".to_string())).await;
    assert_eq!(
        router.resolve("127.0.0.1").await,
        Resolution::App {
            key: "app2:production".to_string(),
            target: target_app2_prod
        }
    );
    assert_eq!(
        router.resolve("127.0.0.1:80").await,
        Resolution::App {
            key: "app2:production".to_string(),
            target: target_app2_prod
        }
    );
    assert_eq!(
        router.resolve("::1").await,
        Resolution::App {
            key: "app2:production".to_string(),
            target: target_app2_prod
        }
    );
    assert_eq!(
        router.resolve("[::1]:8080").await,
        Resolution::App {
            key: "app2:production".to_string(),
            target: target_app2_prod
        }
    );
    assert_eq!(
        router.resolve("").await,
        Resolution::App {
            key: "app2:production".to_string(),
            target: target_app2_prod
        }
    );

    // 5. IP default preview (when production route is absent)
    router.remove_route("app2:production").await;
    assert_eq!(
        router.resolve("127.0.0.1").await,
        Resolution::App {
            key: "app2:preview".to_string(),
            target: target_app2_prev
        }
    );
    assert_eq!(
        router.resolve("").await,
        Resolution::App {
            key: "app2:preview".to_string(),
            target: target_app2_prev
        }
    );

    // 6. NotFound
    assert_eq!(
        router.resolve("random.unknown.domain").await,
        Resolution::NotFound
    );

    router.remove_route("app2:preview").await;
    assert_eq!(router.resolve("127.0.0.1").await, Resolution::NotFound);

    router.set_default_app(None).await;
    assert_eq!(router.resolve("127.0.0.1").await, Resolution::NotFound);
    assert_eq!(router.resolve("").await, Resolution::NotFound);
}

#[tokio::test]
async fn test_upsert_replaces_previous_and_rebuilds_host_map() {
    let router = ProxyRouter::new();

    let target1: SocketAddr = "127.0.0.1:4001".parse().unwrap();
    let target2: SocketAddr = "127.0.0.1:4002".parse().unwrap();

    let prev = router
        .upsert_route(Route {
            app: "store".to_string(),
            env: "prod".to_string(),
            hosts: vec!["store.com".to_string(), "shop.com".to_string()],
            target: target1,
            version: "v1".to_string(),
        })
        .await
        .unwrap();
    assert!(prev.is_none());

    assert_eq!(
        router.resolve("store.com").await,
        Resolution::App {
            key: "store:prod".to_string(),
            target: target1
        }
    );
    assert_eq!(
        router.resolve("shop.com").await,
        Resolution::App {
            key: "store:prod".to_string(),
            target: target1
        }
    );

    // Replace store:prod with new hosts (store.com removed, market.com added)
    let prev2 = router
        .upsert_route(Route {
            app: "store".to_string(),
            env: "prod".to_string(),
            hosts: vec!["shop.com".to_string(), "market.com".to_string()],
            target: target2,
            version: "v2".to_string(),
        })
        .await
        .unwrap();

    assert!(prev2.is_some());
    assert_eq!(prev2.unwrap().version, "v1");

    // Old host "store.com" is now NotFound
    assert_eq!(router.resolve("store.com").await, Resolution::NotFound);

    // Remaining and new hosts resolve to target2
    assert_eq!(
        router.resolve("shop.com").await,
        Resolution::App {
            key: "store:prod".to_string(),
            target: target2
        }
    );
    assert_eq!(
        router.resolve("market.com").await,
        Resolution::App {
            key: "store:prod".to_string(),
            target: target2
        }
    );
}

#[tokio::test]
async fn test_health_checker() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
    let checker = HealthChecker::new(Duration::from_millis(500));
    let status = checker.probe(addr, "/healthz").await.unwrap();
    assert_eq!(status, 200);

    let wait_res = checker
        .wait_for_healthy(addr, "/healthz", Duration::from_millis(500))
        .await;
    assert!(wait_res.is_ok());
}
#[tokio::test]
async fn test_route_listing_control_host_rejected() {
    let router = ProxyRouter::new();
    let control_addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
    let app_addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();

    router
        .set_control(Some("skoci.example.com".to_string()), control_addr)
        .await;

    // Route attempting to claim the control host is rejected
    let res = router
        .upsert_route(Route {
            app: "attacker".to_string(),
            env: "preview".to_string(),
            hosts: vec!["skoci.example.com".to_string()],
            target: app_addr,
            version: "v1".to_string(),
        })
        .await;

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "conflict");
    assert!(err.message.contains("reserved for the control plane"));

    // Control host still resolves to control
    assert_eq!(
        router.resolve("skoci.example.com").await,
        Resolution::Control(control_addr)
    );
}

#[tokio::test]
async fn test_cross_app_host_hijack_rejected() {
    let router = ProxyRouter::new();
    let addr1: SocketAddr = "127.0.0.1:3001".parse().unwrap();
    let addr2: SocketAddr = "127.0.0.1:3002".parse().unwrap();

    // App 1 claims victim.example.com
    router
        .upsert_route(Route {
            app: "victim".to_string(),
            env: "production".to_string(),
            hosts: vec!["victim.example.com".to_string()],
            target: addr1,
            version: "v1".to_string(),
        })
        .await
        .unwrap();

    // App 2 tries to claim the same host
    let res = router
        .upsert_route(Route {
            app: "attacker".to_string(),
            env: "production".to_string(),
            hosts: vec!["victim.example.com".to_string()],
            target: addr2,
            version: "v1".to_string(),
        })
        .await;

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "conflict");

    // Same app other env also rejected
    let res_same_app = router
        .upsert_route(Route {
            app: "victim".to_string(),
            env: "preview".to_string(),
            hosts: vec!["victim.example.com".to_string()],
            target: addr2,
            version: "v1".to_string(),
        })
        .await;

    assert!(res_same_app.is_err());
    assert_eq!(res_same_app.unwrap_err().code, "conflict");

    // Victim still owns the host
    assert_eq!(
        router.resolve("victim.example.com").await,
        Resolution::App {
            key: "victim:production".to_string(),
            target: addr1,
        }
    );

    assert_eq!(
        router.host_owner("victim.example.com").await,
        Some("victim:production".to_string())
    );
}

#[test]
fn test_decide_routing_authority_unit() {
    use skoci_proxy::{decide_routing_authority, RoutingAuthorityError};

    // 1. URI authority only
    let dec = decide_routing_authority(Some("app.example.com"), None, false, None).unwrap();
    assert_eq!(dec.clean_host, "app.example.com");
    assert_eq!(dec.forward_host, "app.example.com");

    // 2. Host header only
    let dec = decide_routing_authority(None, Some("app.example.com"), false, None).unwrap();
    assert_eq!(dec.clean_host, "app.example.com");
    assert_eq!(dec.forward_host, "app.example.com");

    // 3. Both present and matching
    let dec = decide_routing_authority(
        Some("app.example.com:8080"),
        Some("app.example.com:8080"),
        false,
        None,
    )
    .unwrap();
    assert_eq!(dec.clean_host, "app.example.com");
    assert_eq!(dec.forward_host, "app.example.com:8080");

    // 4. Both present and case-insensitively matching
    let dec = decide_routing_authority(
        Some("APP.example.com"),
        Some("app.example.com"),
        false,
        None,
    )
    .unwrap();
    assert_eq!(dec.clean_host, "app.example.com");

    // 5. Both present and different -> 400 BadRequest
    let err = decide_routing_authority(
        Some("app1.example.com"),
        Some("app2.example.com"),
        false,
        None,
    )
    .unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::BadRequest("URI authority and Host header do not match")
    );

    // 6. Userinfo in authority -> 400 BadRequest
    let err = decide_routing_authority(None, Some("a.example:@evil.example"), false, None)
        .unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::BadRequest("Invalid host or authority syntax")
    );

    // 7. Plain HTTP: empty host -> allowed (for default app fallback)
    let dec = decide_routing_authority(None, None, false, None).unwrap();
    assert_eq!(dec.clean_host, "");
    assert_eq!(dec.forward_host, "localhost");

    // 8. Plain HTTP: IP host -> allowed
    let dec = decide_routing_authority(None, Some("127.0.0.1"), false, None).unwrap();
    assert_eq!(dec.clean_host, "127.0.0.1");

    // 9. HTTPS: empty host -> 421 MisdirectedRequest
    let err = decide_routing_authority(None, None, true, Some("example.com")).unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::MisdirectedRequest("Empty or IP authority forbidden on HTTPS")
    );

    // 10. HTTPS: IP host -> 421 MisdirectedRequest
    let err =
        decide_routing_authority(None, Some("127.0.0.1"), true, Some("127.0.0.1")).unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::MisdirectedRequest("Empty or IP authority forbidden on HTTPS")
    );

    // 11. HTTPS: missing SNI -> 421 MisdirectedRequest
    let err = decide_routing_authority(None, Some("example.com"), true, None).unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::MisdirectedRequest("Missing TLS SNI on HTTPS request")
    );

    // 12. HTTPS: SNI mismatch -> 421 MisdirectedRequest
    let err = decide_routing_authority(
        None,
        Some("victim.com"),
        true,
        Some("attacker.com"),
    )
    .unwrap_err();
    assert_eq!(
        err,
        RoutingAuthorityError::MisdirectedRequest("Authority does not match TLS SNI")
    );

    // 13. HTTPS: SNI match (case-insensitive, port ignored) -> Ok
    let dec = decide_routing_authority(
        None,
        Some("EXPLICIT.COM:443"),
        true,
        Some("explicit.com"),
    )
    .unwrap();
    assert_eq!(dec.clean_host, "explicit.com");
    assert_eq!(dec.forward_host, "EXPLICIT.COM:443");
}
