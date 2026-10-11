use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "skoci", about = "Lightweight agent-native deployment platform", version)]
pub struct Cli {
    #[arg(long, global = true, help = "Output formatted JSON")]
    pub json: bool,

    #[arg(long, global = true, env = "SKOCI_SERVER_URL", default_value = "http://127.0.0.1:8787")]
    pub server_url: String,

    #[arg(long, global = true, env = "SKOCI_TOKEN")]
    pub token: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Detect project framework and create skoci.yaml & Dockerfile")]
    Init {
        #[arg(short, long, help = "App name (defaults to current directory name)")]
        app: Option<String>,
        #[arg(default_value = ".")]
        path: String,
    },

    #[command(about = "Build, deploy, and verify application")]
    Deploy {
        #[arg(help = "Path to skoci.yaml config (optional)")]
        config: Option<String>,
        #[arg(short, long, default_value = "preview", help = "Target environment: preview or production")]
        env: String,
        #[arg(long, help = "Force deployment even if policy requires confirmation")]
        force: bool,
    },

    #[command(about = "Show application and service health status")]
    Status {
        #[arg(help = "App name (reads from skoci.yaml if omitted)")]
        app: Option<String>,
    },

    #[command(about = "Show recent application logs with error extraction")]
    Logs {
        #[arg(help = "App name (reads from skoci.yaml if omitted)")]
        app: Option<String>,
        #[arg(short, long, default_value_t = 100)]
        tail: usize,
    },

    #[command(about = "Manage encrypted secrets")]
    Secret {
        #[command(subcommand)]
        command: SecretCommands,
    },

    #[command(about = "Request promotion to production")]
    Promote {
        #[arg(help = "Reason for promotion")]
        reason: String,
        #[arg(short, long, default_value = "production")]
        env: String,
    },

    #[command(about = "Approve pending promotion (human only)")]
    Approve {
        #[arg(help = "Approval request ID")]
        request_id: String,
    },

    #[command(about = "Rollback application to last healthy version")]
    Rollback {
        #[arg(help = "Target version (defaults to previous healthy)")]
        target: Option<String>,
    },

    #[command(about = "Check server environment, idle RAM, and Docker status")]
    Doctor,

    #[command(about = "Database backup and restore")]
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },

    #[command(about = "Manage scoped agent tokens (cross-project isolation)")]
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },

    #[command(about = "Generate machine-readable guidelines and runtime summary for AI agents")]
    AgentGuide,

    #[command(about = "Generate systemd unit file for 24/7 VPS operation")]
    InstallService {
        #[arg(long, default_value = "skoci", help = "Systemd service name")]
        service_name: String,
        #[arg(long, default_value = "127.0.0.1:8787", help = "Bind address")]
        bind: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecretCommands {
    #[command(about = "Store secret encrypted at rest")]
    Set {
        key: String,
        value: String,
    },
    #[command(about = "List secret keys (values remain hidden)")]
    List,
}

#[derive(Subcommand, Debug)]
pub enum DbCommands {
    #[command(about = "Create database backup immediately")]
    Backup {
        #[arg(default_value = "db")]
        service: String,
    },
    #[command(about = "Restore database from backup")]
    Restore {
        backup_id: String,
        #[arg(default_value = "db")]
        service: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum TokenCommands {
    #[command(about = "Create a scoped agent token locked to a specific application")]
    Create {
        #[arg(short, long, help = "Application name this token is locked to")]
        app: String,
        #[arg(short, long, default_value = "agent", help = "Friendly token label")]
        name: String,
    },
    #[command(about = "List active tokens for an app")]
    List {
        #[arg(short, long, help = "Application name")]
        app: String,
    },
}
