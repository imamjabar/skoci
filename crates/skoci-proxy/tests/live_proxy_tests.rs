use skoci_proxy::router::{ProxyRouter, Route};
use skoci_proxy::server::{start_proxy, ProxyConfig};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct UpstreamState {
    last_headers: HashMap<String, String>,
    last_body: Vec<u8>,
}

async fn spawn_raw_upstream() -> (SocketAddr, Arc<Mutex<UpstreamState>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(UpstreamState {
        last_headers: HashMap::new(),
        last_body: Vec::new(),
    }));
    let state_clone = state.clone();

    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            let state = state_clone.clone();

            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end;
                loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = pos + 4;
                        break;
                    }
                }

                let header_str = String::from_utf8_lossy(&buf[..header_end]);
                let mut lines = header_str.split("\r\n");
                let req_line = lines.next().unwrap_or("");
                let mut req_parts = req_line.split_whitespace();
                let _method = req_parts.next().unwrap_or("GET");
                let path = req_parts.next().unwrap_or("/").to_string();

                let mut headers = HashMap::new();
                let mut content_length = 0usize;
                for line in lines {
                    if line.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        let key = k.trim().to_ascii_lowercase();
                        let val = v.trim().to_string();
                        if key == "content-length" {
                            content_length = val.parse().unwrap_or(0);
                        }
                        headers.insert(key, val);
                    }
                }

                let mut body = buf[header_end..].to_vec();
                while body.len() < content_length {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }

                {
                    let mut st = state.lock().await;
                    st.last_headers = headers.clone();
                    st.last_body = body.clone();
                }

                if path == "/test-headers" {
                    let resp = "HTTP/1.1 200 OK\r\n\
                                Content-Type: application/custom-type; charset=utf-8\r\n\
                                X-Custom-Header: skoci-preserved\r\n\
                                Content-Length: 14\r\n\
                                \r\n\
                                Header Content";
                    let _ = stream.write_all(resp.as_bytes()).await;
                } else if path == "/test-post" {
                    let body_len = body.len();
                    let resp_header = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/octet-stream\r\n\
                         Content-Length: {body_len}\r\n\
                         \r\n"
                    );
                    let _ = stream.write_all(resp_header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                } else if path == "/test-large-stream" {
                    let size = 2 * 1024 * 1024; // 2 MB
                    let resp_header = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/octet-stream\r\n\
                         Content-Length: {size}\r\n\
                         \r\n"
                    );
                    let _ = stream.write_all(resp_header.as_bytes()).await;
                    let chunk_pattern = [b'X'; 65536];
                    let mut sent = 0;
                    while sent < size {
                        let to_send = (size - sent).min(chunk_pattern.len());
                        let _ = stream.write_all(&chunk_pattern[..to_send]).await;
                        sent += to_send;
                    }
                } else {
                    let resp = "HTTP/1.1 200 OK\r\n\
                                Content-Type: text/plain\r\n\
                                Content-Length: 2\r\n\
                                \r\n\
                                OK";
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    (addr, state)
}

#[tokio::test]
async fn test_live_proxy_end_to_end() {
    let (upstream_addr, upstream_state) = spawn_raw_upstream().await;

    let router = ProxyRouter::new();
    router
        .upsert_route(Route {
            app: "my-app".to_string(),
            env: "production".to_string(),
            hosts: vec!["my-app.local".to_string()],
            target: upstream_addr,
            version: "v1".to_string(),
        })
        .await
        .unwrap();

    let control_addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
    router
        .set_control(Some("control.skoci.local".to_string()), control_addr)
        .await;

    let cfg = ProxyConfig {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        https_bind: "127.0.0.1:0".parse().unwrap(),
        acme: None,
    };

    let status = start_proxy(cfg, router.clone()).await;
    assert!(status.http_listening());
    assert!(!status.tls_enabled());
    assert!(status.bind_errors().is_empty());

    let proxy_http_addr = status.http_addr().expect("proxy http addr must be bound");
    let client = reqwest::Client::new();

    // 1. Response headers like Content-Type are preserved
    let resp = client
        .get(format!("http://{proxy_http_addr}/test-headers"))
        .header("Host", "my-app.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/custom-type; charset=utf-8"
    );
    assert_eq!(
        resp.headers().get("x-custom-header").unwrap(),
        "skoci-preserved"
    );
    assert!(resp.headers().contains_key("x-request-id"));
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Header Content");

    // 2. X-Forwarded-* headers set on upstream
    {
        let st = upstream_state.lock().await;
        assert_eq!(
            st.last_headers.get("x-forwarded-host").map(|s| s.as_str()),
            Some("my-app.local")
        );
        assert_eq!(
            st.last_headers.get("x-forwarded-proto").map(|s| s.as_str()),
            Some("http")
        );
        assert!(st.last_headers.contains_key("x-forwarded-for"));
        assert!(st.last_headers.contains_key("x-request-id"));
    }

    // 3. POST body forwarded correctly
    let post_data = vec![b'A'; 256 * 1024]; // 256 KB
    let resp_post = client
        .post(format!("http://{proxy_http_addr}/test-post"))
        .header("Host", "my-app.local")
        .body(post_data.clone())
        .send()
        .await
        .unwrap();

    assert_eq!(resp_post.status(), 200);
    let echoed_bytes = resp_post.bytes().await.unwrap();
    assert_eq!(echoed_bytes.len(), post_data.len());
    assert_eq!(&echoed_bytes[..], &post_data[..]);

    {
        let st = upstream_state.lock().await;
        assert_eq!(st.last_body.len(), post_data.len());
        assert_eq!(&st.last_body[..], &post_data[..]);
    }

    // 4. Streaming / large body passes
    let resp_large = client
        .get(format!("http://{proxy_http_addr}/test-large-stream"))
        .header("Host", "my-app.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp_large.status(), 200);
    let large_bytes = resp_large.bytes().await.unwrap();
    assert_eq!(large_bytes.len(), 2 * 1024 * 1024);
    assert!(large_bytes.iter().all(|&b| b == b'X'));

    // 5. Unknown host -> 404 with text and hint
    let resp_404 = client
        .get(format!("http://{proxy_http_addr}/"))
        .header("Host", "unknown-domain.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp_404.status(), 404);
    let text_404 = resp_404.text().await.unwrap();
    assert!(
        text_404.contains("404 Not Found"),
        "body was: {text_404}"
    );
    assert!(
        text_404.contains("add the domain to skoci.yaml then run skoci deploy"),
        "body was: {text_404}"
    );

    // 6. Upstream down -> 502 with hint skoci logs
    // Bind a listener then drop it immediately to get a guaranteed down port
    let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener);

    router
        .upsert_route(Route {
            app: "dead-app".to_string(),
            env: "production".to_string(),
            hosts: vec!["dead-app.local".to_string()],
            target: dead_addr,
            version: "v1".to_string(),
        })
        .await
        .unwrap();

    let resp_502 = client
        .get(format!("http://{proxy_http_addr}/"))
        .header("Host", "dead-app.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp_502.status(), 502);
    let text_502 = resp_502.text().await.unwrap();
    assert!(
        text_502.contains("502 Bad Gateway"),
        "body was: {text_502}"
    );
    assert!(text_502.contains("skoci logs"), "body was: {text_502}");

    // 7. Control host over HTTP when TLS is off -> 403 with exact message
    let resp_403 = client
        .get(format!("http://{proxy_http_addr}/"))
        .header("Host", "control.skoci.local")
        .send()
        .await
        .unwrap();

    assert_eq!(resp_403.status(), 403);
    let text_403 = resp_403.text().await.unwrap();
    let expected_msg = "Public HTTP access to Skoci API is forbidden. Use HTTPS (--tls acme) or an SSH tunnel: ssh -L 8787:127.0.0.1:8787 user@server";
    assert!(
        text_403.contains(expected_msg),
        "body was: {text_403}, expected to contain: {expected_msg}"
    );
}

#[tokio::test]
async fn test_websocket_upgrade_is_tunneled() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = upstream.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut req = Vec::new();
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut buf).await.unwrap();
            req.extend_from_slice(&buf[..n]);
        }
        let head = String::from_utf8_lossy(&req).to_ascii_lowercase();
        assert!(head.contains("upgrade: websocket"));
        s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
            .await
            .unwrap();
        let n = s.read(&mut buf).await.unwrap();
        s.write_all(&buf[..n]).await.unwrap();
    });

    let router = ProxyRouter::new();
    router
        .upsert_route(Route {
            app: "ws".to_string(),
            env: "production".to_string(),
            hosts: vec!["ws.local".to_string()],
            target: upstream_addr,
            version: "v1".to_string(),
        })
        .await
        .unwrap();
    let status = start_proxy(
        ProxyConfig {
            http_bind: "127.0.0.1:0".parse().unwrap(),
            https_bind: "127.0.0.1:0".parse().unwrap(),
            acme: None,
        },
        router,
    )
    .await;
    let proxy_addr = status.http_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET /socket HTTP/1.1\r\nHost: ws.local\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n")
        .await
        .unwrap();
    let mut buf = vec![0u8; 4096];
    let mut resp = Vec::new();
    while !resp.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = client.read(&mut buf).await.unwrap();
        assert!(n > 0, "proxy closed the connection before answering the upgrade");
        resp.extend_from_slice(&buf[..n]);
    }
    assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 101"));

    client.write_all(b"frame-bytes").await.unwrap();
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), client.read(&mut buf))
        .await
        .expect("echo through the upgraded tunnel")
        .unwrap();
    assert_eq!(&buf[..n], b"frame-bytes");
}
