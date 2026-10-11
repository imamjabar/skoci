use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder;
use skoci_proxy::router::ProxyRouter;
use skoci_proxy::server::run_proxy_server;
use skoci_proxy::service::{Service, Target};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;

// Minimal mock target server
async fn spawn_mock_target(port: u16, reply: &'static str) -> SocketAddr {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let service = service_fn(move |_: Request<Incoming>| async move {
                    Ok::<_, hyper::Error>(Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::from(reply)))
                        .unwrap())
                });
                let _ = Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    });

    local_addr
}

#[tokio::test]
async fn test_end_to_end_proxy_forwarding_and_cutover() {
    // 1. Spawn backend target v1 and target v2
    let target1_addr = spawn_mock_target(0, "Hello from v1").await;
    let target2_addr = spawn_mock_target(0, "Hello from v2").await;

    // 2. Setup ProxyRouter
    let router = ProxyRouter::new();
    let service = Service::new("demo-app", vec!["demo.local".to_string()], "/healthz");
    router.register_service(service).await;

    // Set initial target v1
    let t1 = Target {
        id: "c1".to_string(),
        addr: target1_addr,
        version: "v1".to_string(),
        healthy: true,
        last_checked: None,
    };
    router.cutover("demo-app", t1, false).await.unwrap();

    // 3. Spawn Proxy Server on random port
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    drop(proxy_listener); // release so run_proxy_server can bind

    let r_clone = router.clone();
    tokio::spawn(async move {
        let _ = run_proxy_server(proxy_addr, r_clone).await;
    });

    // Brief yield for server startup
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();

    // 4. Test request with Host: demo.local -> should return "Hello from v1"
    let resp1 = client
        .get(format!("http://{proxy_addr}/"))
        .header("Host", "demo.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp1.status(), 200);
    let body1 = resp1.text().await.unwrap();
    assert_eq!(body1, "Hello from v1");

    // 5. Test unknown domain -> should return 404
    let resp_404 = client
        .get(format!("http://{proxy_addr}/"))
        .header("Host", "unknown.local")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_404.status(), 404);

    // 6. Zero-downtime cutover to target v2
    let t2 = Target {
        id: "c2".to_string(),
        addr: target2_addr,
        version: "v2".to_string(),
        healthy: true,
        last_checked: None,
    };
    router.cutover("demo-app", t2, false).await.unwrap();

    // 7. Test request with Host: demo.local -> should now return "Hello from v2"
    let resp2 = client
        .get(format!("http://{proxy_addr}/"))
        .header("Host", "demo.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp2.status(), 200);
    let body2 = resp2.text().await.unwrap();
    assert_eq!(body2, "Hello from v2");
}
