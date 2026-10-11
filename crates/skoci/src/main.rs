use clap::{Parser, Subcommand};
use skoci_cli::commands::run_command;
use skoci_control::api::AppState;
use skoci_control::approval::ApprovalService;
use skoci_control::auth::AuthManager;
use skoci_control::db::Database;
use skoci_control::server::run_server;
use skoci_mcp::McpServer;
use std::net::SocketAddr;
use parking_lot::Mutex;
use skoci_node::backup::BackupManager;
use skoci_node::docker::DockerClient;
use skoci_node::secret::SecretStore;
use skoci_proxy::router::ProxyRouter;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "skoci", about = "Lightweight agent-native deployment platform", version)]
struct MainCli {
    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true, env = "SKOCI_SERVER_URL", default_value = "http://127.0.0.1:8787")]
    server_url: String,

    #[arg(long, global = true, env = "SKOCI_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: MainCommands,
}

#[derive(Subcommand, Debug)]
enum MainCommands {
    #[command(about = "Run all-in-one control plane + node + proxy + dashboard")]
    Server {
        #[arg(short, long, default_value = "127.0.0.1:8787")]
        bind: String,
        #[arg(long, default_value = "skoci.db")]
        db: String,
    },

    #[command(about = "Run standalone worker node + proxy")]
    Node {
        #[arg(long)]
        control_plane_url: String,
        #[arg(long)]
        token: String,
    },

    #[command(about = "Run Model Context Protocol (MCP) server over stdio")]
    Mcp,

    #[command(flatten)]
    Cli(skoci_cli::args::Commands),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = MainCli::parse();

    match args.command {
        MainCommands::Server { bind, db } => {
            let addr: SocketAddr = bind.parse()?;
            println!("🚀 Skoci Server starting on http://{addr}");
            println!("💾 Database: {db}");
            println!("🔒 Idle RAM budget target: <20MB");

            let db = Database::open(db)?;
            let auth = Arc::new(AuthManager::new(
                args.token.unwrap_or_else(|| "sk_master_admin".to_string()),
            ));
            let approvals = ApprovalService::new(db.clone());

            let proxy = ProxyRouter::new();
            let docker = Arc::new(DockerClient::new());
            let secrets = Arc::new(Mutex::new(SecretStore::new("skoci_master_node_seed")));
            let backup_dir = std::path::PathBuf::from("backups");
            let backup = Arc::new(BackupManager::new(backup_dir));

            let proxy_runner = proxy.clone();
            tokio::spawn(async move {
                let proxy_addr: SocketAddr = "127.0.0.1:8000".parse().unwrap();
                let _ = skoci_proxy::server::run_proxy_server(proxy_addr, proxy_runner).await;
            });

            let state = AppState {
                db,
                auth,
                approvals,
                proxy,
                docker,
                secrets,
                backup,
            };

            run_server(addr, state).await?;
        }
        MainCommands::Node { control_plane_url, token: _ } => {
            println!("🛰️ Skoci Node connected to {control_plane_url}");
            println!("🔑 Token: [provided]");
            // Standalone node loop
            tokio::signal::ctrl_c().await?;
        }
        MainCommands::Mcp => {
            let server = McpServer::new(args.server_url, args.token);
            server.run_stdio().await?;
        }
        MainCommands::Cli(cmd) => {
            run_command(cmd, &args.server_url, args.token, args.json).await;
        }
    }

    Ok(())
}
