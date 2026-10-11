use crate::router::ProxyRouter;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

pub async fn run_proxy_server(addr: SocketAddr, router: ProxyRouter) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Skoci reverse proxy listening on http://{addr}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let client = Arc::new(client);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };

        let io = TokioIo::new(stream);
        let router = router.clone();
        let client = client.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let router = router.clone();
                let client = client.clone();
                async move { handle_proxy_request(req, router, client).await }
            });

            let builder = Builder::new(hyper_util::rt::TokioExecutor::new());
            let _ = builder.serve_connection(io, service).await;
        });
    }
}

async fn handle_proxy_request(
    req: Request<Incoming>,
    router: ProxyRouter,
    client: Arc<reqwest::Client>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // 1. Extract host header
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let service_name = match router.resolve_host(&host).await {
        Some(s) => s,
        None => {
            let body = format!("404 Not Found: Domain '{host}' tidak terdaftar di Skoci.\n");
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from(body)))
                .unwrap());
        }
    };

    // 2. Resolve active target (with cutover pause waiting)
    let target_addr = match router.get_target_for_request(&service_name).await {
        Ok(Some(addr)) => addr,
        Ok(None) => {
            // Maintenance mode
            let body = format!("503 Service Unavailable: Aplikasi '{service_name}' sedang dalam mode maintenance.\n");
            return Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Full::new(Bytes::from(body)))
                .unwrap());
        }
        Err(e) => {
            let body = format!("502 Bad Gateway: {e}\n");
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Full::new(Bytes::from(body)))
                .unwrap());
        }
    };

    // 3. Forward request to container target
    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let forward_url = format!("http://{target_addr}{path_and_query}");

    let method = req.method().clone();
    let body_bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => Bytes::new(),
    };

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => reqwest::Method::GET,
    };

    let request_id = format!("req_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());

    let resp = client
        .request(reqwest_method, &forward_url)
        .header("X-Request-Id", &request_id)
        .header("X-Forwarded-Proto", "http")
        .header("X-Forwarded-Host", host)
        .body(body_bytes)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::OK);
            let bytes = r.bytes().await.unwrap_or_default();
            Ok(Response::builder()
                .status(status)
                .header("X-Request-Id", request_id)
                .header("Server", "Skoci-Proxy")
                .body(Full::new(bytes))
                .unwrap())
        }
        Err(e) => {
            let err_body = format!("502 Bad Gateway: Gagal menghubungi container {target_addr}: {e}\n");
            Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Full::new(Bytes::from(err_body)))
                .unwrap())
        }
    }
}
