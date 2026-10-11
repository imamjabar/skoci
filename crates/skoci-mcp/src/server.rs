use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::tools::{call_tool, list_tools};
use serde_json::json;
use skoci_client::SkociClient;
use std::io::{self, BufRead, Write};

pub struct McpServer {
    client: SkociClient,
}

impl McpServer {
    pub fn new(server_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            client: SkociClient::new(server_url, token),
        }
    }

    pub async fn run_stdio(&self) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            if line.trim().is_empty() {
                continue;
            }

            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let err_resp = JsonRpcResponse::err(None, -32700, format!("Parse error: {e}"));
                    writeln!(stdout, "{}", serde_json::to_string(&err_resp)?)?;
                    stdout.flush()?;
                    continue;
                }
            };

            let resp = self.handle_request(req).await;
            writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
            stdout.flush()?;
        }

        Ok(())
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "skoci-mcp",
                        "version": "0.1.0"
                    }
                });
                JsonRpcResponse::ok(req.id, result)
            }
            "tools/list" => {
                let tools = list_tools();
                JsonRpcResponse::ok(req.id, json!({ "tools": tools }))
            }
            "tools/call" => {
                let params = req.params.unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));

                let tool_res = call_tool(&self.client, tool_name, args).await;
                JsonRpcResponse::ok(req.id, json!(tool_res))
            }
            _ => JsonRpcResponse::err(req.id, -32601, format!("Method not found: {}", req.method)),
        }
    }
}
