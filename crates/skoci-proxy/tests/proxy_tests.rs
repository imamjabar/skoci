use skoci_proxy::pause::PauseController;
use skoci_proxy::router::ProxyRouter;
use skoci_proxy::service::{Service, Target};
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn test_pause_controller() {
    let controller = PauseController::new();
    assert!(!controller.is_paused());

    controller.pause();
    assert!(controller.is_paused());

    let c_clone = controller.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c_clone.resume();
    });

    let ok = controller.wait_if_paused(Duration::from_millis(200)).await;
    assert!(ok);
    assert!(!controller.is_paused());
}

#[tokio::test]
async fn test_proxy_router_resolution_and_cutover() {
    let router = ProxyRouter::new();
    let service = Service::new("toko", vec!["toko.example.com".to_string()], "/healthz");
    router.register_service(service).await;

    // Resolve host
    let resolved = router.resolve_host("toko.example.com").await;
    assert_eq!(resolved.as_deref(), Some("toko"));

    // Add target and cutover without health check
    let addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
    let target = Target {
        id: "c1".to_string(),
        addr,
        version: "v1".to_string(),
        healthy: true,
        last_checked: None,
    };

    let prev = router.cutover("toko", target, false).await.unwrap();
    assert!(prev.is_none());

    let target_addr = router.get_target_for_request("toko").await.unwrap();
    assert_eq!(target_addr, Some(addr));

    // Swap to v2
    let addr2: SocketAddr = "127.0.0.1:3001".parse().unwrap();
    let target2 = Target {
        id: "c2".to_string(),
        addr: addr2,
        version: "v2".to_string(),
        healthy: true,
        last_checked: None,
    };
    let prev2 = router.cutover("toko", target2, false).await.unwrap();
    assert_eq!(prev2.unwrap().version, "v1");

    let target_addr2 = router.get_target_for_request("toko").await.unwrap();
    assert_eq!(target_addr2, Some(addr2));
}
