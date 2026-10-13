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

    #[arg(long, global = true, env = "SKOCI_SERVER_URL", help = "Control plane URL; defaults to the saved profile, else http://127.0.0.1:8787")]
    server_url: Option<String>,

    #[arg(long, global = true, env = "SKOCI_TOKEN", hide_env_values = true)]
    token: Option<String>,

    #[arg(long, global = true, env = "SKOCI_SERVER_PIN", help = "Pinned server TLS key, sha256:<hex>, required for https://<ip> targets")]
    pin: Option<String>,

    #[arg(long, global = true, env = "SKOCI_PROFILE", help = "Named server profile from ~/.config/skoci/servers.json")]
    profile: Option<String>,

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
    #[command(about = "Server lifecycle: run the daemon, install a VPS, read the TLS pin")]
    Server {
        #[command(subcommand)]
        cmd: ServerCommands,
    },

    #[command(about = "Run Model Context Protocol (MCP) server over stdio")]
    Mcp,

    #[command(flatten)]
    Cli(skoci_cli::args::Commands),
}

#[derive(Subcommand, Debug)]
enum ServerCommands {
    #[command(about = "Run control plane + builder + proxy + dashboard (on the VPS)")]
    Run(Box<RunArgs>),

    #[command(about = "Install Skoci on a remote VPS over SSH and save a pinned profile")]
    Setup {
        #[arg(long, help = "VPS address (IP or hostname)")]
        host: String,
        #[arg(long, default_value = "root")]
        user: String,
        #[arg(long, default_value_t = 22)]
        port: u16,
        #[arg(long, help = "SSH private key to authenticate with")]
        ssh_key: Option<PathBuf>,
        #[arg(long, help = "Bootstrap SSH public key to server using ssh-copy-id if password auth is required")]
        ssh_copy_id: bool,
        #[arg(long, help = "Read SSH/sudo password from first line of stdin for non-interactive bootstrap")]
        ssh_password_stdin: bool,
        #[arg(long, help = "App the generated token is scoped to (default: app from skoci.yaml)")]
        app: Option<String>,
        #[arg(long, help = "Skoci version tag to install (default: latest)")]
        version: Option<String>,
        #[arg(long, help = "Enable sslip.io preview hostnames; unsuitable for production")]
        magic_dns: bool,
        #[arg(long, help = "Base domain you own, used for app hostnames and TLS")]
        domain: Option<String>,
        #[arg(long, help = "Contact address for Let's Encrypt")]
        email: Option<String>,
        #[arg(long, help = "Disable password authentication and root passwords in sshd")]
        harden_ssh: bool,
        #[arg(long, help = "Skip UFW firewall configuration (UFW is configured by default)")]
        no_ufw: bool,
        /// Require human approval for every production promotion, including an app's first one
        #[arg(long, help = "Require human approval for every production promotion, including an app's first one")]
        strict_approvals: bool,
        #[arg(long, short = 'y', help = "Do not prompt for confirmation")]
        yes: bool,
        #[arg(long, help = "Operator override: allow sslip.io-style hostnames in production")]
        allow_magic_dns_production: bool,
    },

    #[command(about = "Print this server's TLS key pin (run on the VPS)")]
    Fingerprint,
}

#[derive(clap::Args, Debug)]
struct RunArgs {
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
    #[arg(
        long,
        env = "SKOCI_ACME_STAGING",
        value_parser = parse_loose_bool,
        num_args = 0..=1,
        default_missing_value = "true",
        default_value = "false"
    )]
    acme_staging: bool,
    #[arg(long, env = "SKOCI_MAX_UPLOAD_MB", default_value_t = 200)]
    max_upload_mb: u64,
    #[arg(long, env = "SKOCI_PUBLIC_IP", help = "Public server IP: raw-IP routing, IP certificate, DNS checks")]
    public_ip: Option<IpAddr>,
    #[arg(long, env = "SKOCI_CONTROL_TLS_BIND", default_value = "0.0.0.0:8443", help = "Pinned TLS control endpoint; 'off' disables it")]
    control_tls_bind: String,
    #[arg(
        long,
        env = "SKOCI_ALLOW_MAGIC_DNS_PRODUCTION",
        value_parser = parse_loose_bool,
        num_args = 0..=1,
        default_missing_value = "true",
        default_value = "false",
        help = "Operator override: allow sslip.io-style hostnames in production"
    )]
    allow_magic_dns_production: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    skoci_cli::env::load_env_files();
    let args = MainCli::parse();

    let default_filter = if matches!(args.command, MainCommands::Server { cmd: ServerCommands::Run(_) }) {
        "info"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    match args.command {
        MainCommands::Server {
            cmd: ServerCommands::Run(run),
        } => {
            let master_token = std::env::var("SKOCI_MASTER_TOKEN").unwrap_or_default();
            std::env::remove_var("SKOCI_MASTER_TOKEN");
            let require_approval_always = std::env::var("SKOCI_REQUIRE_APPROVAL_ALWAYS")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|v| parse_loose_bool(&v))
                .transpose()
                .map_err(|e| anyhow::anyhow!("Invalid SKOCI_REQUIRE_APPROVAL_ALWAYS: {e}"))?
                .unwrap_or(false);
            let base_domain = run.base_domain.filter(|d| !d.trim().is_empty());
            let api_domain = run
                .api_domain
                .filter(|d| !d.trim().is_empty())
                .or_else(|| base_domain.as_ref().map(|b| format!("skoci.{b}")));
            let cfg = ServerConfig {
                bind: run.bind,
                data_dir: run.data_dir,
                master_token,
                http_bind: run.http_bind,
                https_bind: run.https_bind,
                base_domain,
                api_domain,
                tls: match run.tls {
                    TlsArg::Off => TlsMode::Off,
                    TlsArg::Acme => TlsMode::Acme,
                },
                acme_email: run.acme_email.filter(|e| !e.trim().is_empty()),
                acme_staging: run.acme_staging,
                max_upload_mb: run.max_upload_mb,
                public_ip: run.public_ip,
                control_tls_bind: parse_control_tls_bind(&run.control_tls_bind)?,
                allow_magic_dns_production: run.allow_magic_dns_production,
                require_approval_always,
            };
            skoci_control::server::run_server(cfg).await?;
        }
        MainCommands::Server {
            cmd:
                ServerCommands::Setup {
                    host,
                    user,
                    port,
                    ssh_key,
                    ssh_copy_id,
                    ssh_password_stdin,
                    app,
                    version,
                    magic_dns,
                    domain,
                    email,
                    harden_ssh,
                    no_ufw,
                    strict_approvals,
                    yes,
                    allow_magic_dns_production,
                },
        } => {
            let opts = skoci_cli::setup::SetupOptions {
                host,
                user,
                port,
                ssh_key,
                ssh_copy_id,
                ssh_password_stdin,
                app,
                version,
                magic_dns,
                domain,
                email,
                harden_ssh,
                no_ufw,
                strict_approvals,
                assume_yes: yes,
                allow_magic_dns_production,
            };
            let code = skoci_cli::setup::run_server_setup(opts, args.json).await;
            std::process::exit(code);
        }
        MainCommands::Server {
            cmd: ServerCommands::Fingerprint,
        } => {
            let code = skoci_cli::setup::run_server_fingerprint(args.json).await;
            std::process::exit(code);
        }
        MainCommands::Mcp => {
            let global = skoci_cli::commands::GlobalOpts {
                server_url: args.server_url,
                token: args.token,
                pin: args.pin,
                profile: args.profile,
            };
            let target = skoci_cli::commands::resolve_global(&global)?;
            McpServer::new(target.url, target.token, target.pin)?
                .run_stdio()
                .await?;
        }
        MainCommands::Cli(cmd) => {
            let global = skoci_cli::commands::GlobalOpts {
                server_url: args.server_url,
                token: args.token,
                pin: args.pin,
                profile: args.profile,
            };
            let code = skoci_cli::commands::run_command(cmd, global, args.json).await;
            std::process::exit(code);
        }
    }

    Ok(())
}

fn parse_control_tls_bind(value: &str) -> anyhow::Result<Option<SocketAddr>> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("off") || trimmed.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let addr = trimmed
        .parse::<SocketAddr>()
        .map_err(|e| anyhow::anyhow!("Invalid --control-tls-bind '{trimmed}': {e}. Use host:port such as 0.0.0.0:8443, or 'off'"))?;
    Ok(Some(addr))
}

fn parse_loose_bool(raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "invalid boolean '{raw}'; accepted values: true/false, 1/0, yes/no, on/off"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_loose_bool_accepted_values() {
        for truthy in ["true", "True", "TRUE", "1", "yes", "YES", "Yes", "on", "ON", "On", "  true  ", " 1 "] {
            assert_eq!(parse_loose_bool(truthy), Ok(true), "failed on truthy: {truthy}");
        }
        for falsy in ["false", "False", "FALSE", "0", "no", "NO", "No", "off", "OFF", "Off", "  false  ", " 0 "] {
            assert_eq!(parse_loose_bool(falsy), Ok(false), "failed on falsy: {falsy}");
        }
    }

    #[test]
    fn test_parse_loose_bool_rejects_junk() {
        for junk in ["maybe", "2", "-1", "invalid", "", "   ", "none", "null"] {
            let res = parse_loose_bool(junk);
            assert!(res.is_err(), "expected error for junk: '{junk}'");
            let err_msg = res.unwrap_err();
            assert!(
                err_msg.contains("true/false, 1/0, yes/no, on/off"),
                "error message must name accepted values: {err_msg}"
            );
        }
    }
}
