use clap::{Parser, Subcommand, ValueEnum};
use skoci_control::config::{ServerConfig, TlsMode};
use skoci_mcp::McpServer;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "skoci", about = "Lightweight agent-native deployment platform", version)]
struct MainCli {
    #[arg(long, global = true, help = "Output JSON (ApiResponse) only")]
    json: bool,

    #[arg(long, global = true, env = "SKOCI_SERVER_URL", default_value = "http://127.0.0.1:8787")]
    server_url: String,

    #[arg(long, global = true, env = "SKOCI_TOKEN", hide_env_values = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: MainCommands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TlsArg {
    Off,
    Acme,
}

#[derive(Subcommand, Debug)]
enum MainCommands {
    #[command(about = "Run control plane + builder + proxy + dashboard (on the VPS)")]
    Server {
        #[arg(long, env = "SKOCI_BIND", default_value = "127.0.0.1:8787", help = "Control API + dashboard address")]
        bind: SocketAddr,
        #[arg(long, env = "SKOCI_DATA_DIR", default_value = "/var/lib/skoci")]
        data_dir: PathBuf,
        #[arg(long, env = "SKOCI_HTTP_BIND", default_value = "0.0.0.0:80")]
        http_bind: SocketAddr,
        #[arg(long, env = "SKOCI_HTTPS_BIND", default_value = "0.0.0.0:443")]
        https_bind: SocketAddr,
        #[arg(long, env = "SKOCI_BASE_DOMAIN", help = "Auto hostnames: <app>.<base> and <app>-preview.<base>")]
        base_domain: Option<String>,
        #[arg(long, env = "SKOCI_API_DOMAIN", help = "Public hostname for the API/dashboard (default skoci.<base-domain>)")]
        api_domain: Option<String>,
        #[arg(long, env = "SKOCI_TLS", value_enum, default_value = "off")]
        tls: TlsArg,
        #[arg(long, env = "SKOCI_ACME_EMAIL")]
        acme_email: Option<String>,
        #[arg(long, env = "SKOCI_ACME_STAGING")]
        acme_staging: bool,
        #[arg(long, env = "SKOCI_MAX_UPLOAD_MB", default_value_t = 200)]
        max_upload_mb: u64,
        #[arg(long, env = "SKOCI_PUBLIC_IP", help = "Public server IP, reported as the URL of the default app")]
        public_ip: Option<IpAddr>,
    },

    #[command(about = "Run Model Context Protocol (MCP) server over stdio")]
    Mcp,

    #[command(flatten)]
    Cli(skoci_cli::args::Commands),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    skoci_cli::env::load_env_files();
    let args = MainCli::parse();

    let default_filter = if matches!(args.command, MainCommands::Server { .. }) { "info" } else { "warn" };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    match args.command {
        MainCommands::Server {
            bind,
            data_dir,
            http_bind,
            https_bind,
            base_domain,
            api_domain,
            tls,
            acme_email,
            acme_staging,
            max_upload_mb,
            public_ip,
        } => {
            let master_token = std::env::var("SKOCI_MASTER_TOKEN").unwrap_or_default();
            let base_domain = base_domain.filter(|d| !d.trim().is_empty());
            let api_domain = api_domain
                .filter(|d| !d.trim().is_empty())
                .or_else(|| base_domain.as_ref().map(|b| format!("skoci.{b}")));
            let cfg = ServerConfig {
                bind,
                data_dir,
                master_token,
                http_bind,
                https_bind,
                base_domain,
                api_domain,
                tls: match tls {
                    TlsArg::Off => TlsMode::Off,
                    TlsArg::Acme => TlsMode::Acme,
                },
                acme_email: acme_email.filter(|e| !e.trim().is_empty()),
                acme_staging,
                max_upload_mb,
                public_ip,
            };
            skoci_control::server::run_server(cfg).await?;
        }
        MainCommands::Mcp => {
            McpServer::new(args.server_url, args.token).run_stdio().await?;
        }
        MainCommands::Cli(cmd) => {
            let code = skoci_cli::commands::run_command(cmd, &args.server_url, args.token, args.json).await;
            std::process::exit(code);
        }
    }

    Ok(())
}
