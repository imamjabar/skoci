use crate::api::{create_router, AppState};
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub async fn run_server(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = create_router(state);
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Skoci control plane running on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
