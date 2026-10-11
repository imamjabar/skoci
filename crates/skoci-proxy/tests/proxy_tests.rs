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
        .await;

    router
        .upsert_route(Route {
            app: "app2".to_string(),
            env: "preview".to_string(),
            hosts: vec!["preview.app2.org".to_string()],
            target: target_app2_prev,
            version: "v1".to_string(),
        })
        .await;

    router
        .upsert_route(Route {
            app: "app2".to_string(),
            env: "production".to_string(),
            hosts: vec!["app2.org".to_string()],
            target: target_app2_prod,
            version: "v1".to_string(),
        })
        .await;

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

    // 2. Explicit host > control
    router
        .upsert_route(Route {
            app: "override".to_string(),
            env: "production".to_string(),
            hosts: vec!["control.skoci.io".to_string()],
            target: target_override,
            version: "v1".to_string(),
        })
        .await;

    assert_eq!(
        router.resolve("control.skoci.io").await,
        Resolution::App {
            key: "override:production".to_string(),
            target: target_override
        }
    );

    // Remove override to test control resolution
    router.remove_route("override:production").await;

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
        .await;
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
        .await;

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
