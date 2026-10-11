use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    #[command(about = "Connect CLI to Skoci server and save configuration")]
    Connect {
        #[arg(long)]
        server: String,
        #[arg(long)]
        token: String,
        #[arg(long, help = "Allow unencrypted HTTP connection to non-localhost hosts")]
        insecure: bool,
    },

    #[command(about = "Detect framework and initialize project configuration")]
    Init {
        #[arg(long)]
        app: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    #[command(about = "Deploy project to Skoci server")]
    Deploy {
        #[arg(long, default_value = "preview")]
        env: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, help = "Run deployment in the background without waiting for events")]
        detach: bool,
        #[arg(long, help = "Commit SHA or git identifier")]
        commit: Option<String>,
    },

    #[command(about = "Watch live events from a running deployment")]
    Watch {
        deployment_id: String,
    },

    #[command(about = "List deployment history")]
    Deployments {
        #[arg(long)]
        app: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    #[command(about = "View application and container health status")]
    Status {
        app: Option<String>,
    },

    #[command(about = "View application logs and error summary")]
    Logs {
        app: Option<String>,
        #[arg(long)]
        env: Option<String>,
        #[arg(long, default_value = "100")]
        tail: usize,
    },

    #[command(about = "Manage secrets and encrypted variables")]
    Secret {
        #[command(subcommand)]
        action: SecretCommands,
    },

    #[command(about = "Promote preview version to production")]
    Promote {
        reason: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, help = "Run deployment in the background")]
        detach: bool,
    },
    #[command(about = "List pending approval requests")]
    Approvals,

    #[command(about = "Approve an approval request (admin only)")]
    Approve {
        request_id: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, help = "Run deployment in the background")]
        detach: bool,
    },
    #[command(about = "Reject an approval request (admin only)")]
    Reject {
        request_id: String,
    },

    #[command(about = "Roll back application to a previous version")]
    Rollback {
        #[arg(long, default_value = "preview")]
        env: String,
        #[arg(long, help = "Target rollback version")]
        to: Option<String>,
        #[arg(long, help = "Run deployment in the background")]
        detach: bool,
    },
    #[command(about = "Check server, Docker, and configuration health")]
    Doctor,

    #[command(about = "Backup and restore application database")]
    Db {
        #[command(subcommand)]
        action: DbCommands,
    },

    #[command(about = "Manage applications")]
    App {
        #[command(subcommand)]
        action: AppCommands,
    },

    #[command(name = "default-app", about = "View or set default application")]
    DefaultApp {
        app: Option<String>,
    },

    #[command(about = "Manage scoped agent tokens")]
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },

    #[command(name = "agent-guide", about = "Skoci operational guide for AI agents")]
    AgentGuide,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SecretCommands {
    #[command(about = "Set secret for application")]
    Set {
        key: String,
        value: String,
    },
    #[command(about = "List application secret key names")]
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DbCommands {
    #[command(about = "Create database backup")]
    Backup {
        #[arg(default_value = "db")]
        service: String,
    },
    #[command(about = "List database backup history")]
    List,
    #[command(about = "Restore database from backup")]
    Restore {
        backup_id: String,
        #[arg(default_value = "db")]
        service: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    #[command(about = "Delete application")]
    Delete {
        app: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TokenCommands {
    #[command(about = "Create a new scoped agent token")]
    Create {
        #[arg(long)]
        app: String,
        #[arg(long, default_value = "agent")]
        name: String,
    },
    #[command(about = "List tokens for application")]
    List {
        #[arg(long)]
        app: String,
    },
    #[command(about = "Revoke token")]
    Revoke {
        #[arg(long)]
        app: String,
        #[arg(long)]
        name: String,
    },
}
