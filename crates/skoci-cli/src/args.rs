use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    #[command(about = "Hubungkan CLI ke server Skoci dan simpan konfigurasi")]
    Connect {
        #[arg(long)]
        server: String,
        #[arg(long)]
        token: String,
        #[arg(long, help = "Izinkan koneksi HTTP tidak terenkripsi ke host non-localhost")]
        insecure: bool,
    },

    #[command(about = "Deteksi framework dan inisialisasi konfigurasi proyek")]
    Init {
        #[arg(long)]
        app: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    #[command(about = "Deploy proyek ke server Skoci")]
    Deploy {
        #[arg(long, default_value = "preview")]
        env: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, help = "Jalankan deployment di latar belakang tanpa menunggu event")]
        detach: bool,
        #[arg(long, help = "Commit SHA atau identifier git")]
        commit: Option<String>,
    },

    #[command(about = "Pantau live event dari deployment yang sedang berjalan")]
    Watch {
        deployment_id: String,
    },

    #[command(about = "Daftar riwayat deployment")]
    Deployments {
        #[arg(long)]
        app: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    #[command(about = "Lihat status kesehatan aplikasi dan kontainer")]
    Status {
        app: Option<String>,
    },

    #[command(about = "Lihat log aplikasi dan ringkasan error")]
    Logs {
        app: Option<String>,
        #[arg(long)]
        env: Option<String>,
        #[arg(long, default_value = "100")]
        tail: usize,
    },

    #[command(about = "Kelola secret dan variabel terenkripsi")]
    Secret {
        #[command(subcommand)]
        action: SecretCommands,
    },

    #[command(about = "Promosikan versi preview ke production")]
    Promote {
        reason: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, help = "Jalankan deployment di latar belakang")]
        detach: bool,
    },
    #[command(about = "Lihat daftar permintaan persetujuan yang menunggu")]
    Approvals,

    #[command(about = "Setujui permintaan persetujuan (khusus admin)")]
    Approve {
        request_id: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, help = "Jalankan deployment di latar belakang")]
        detach: bool,
    },
    #[command(about = "Tolak permintaan persetujuan (khusus admin)")]
    Reject {
        request_id: String,
    },

    #[command(about = "Rollback aplikasi ke versi sebelumnya")]
    Rollback {
        #[arg(long, default_value = "preview")]
        env: String,
        #[arg(long, help = "Versi target rollback")]
        to: Option<String>,
        #[arg(long, help = "Jalankan deployment di latar belakang")]
        detach: bool,
    },
    #[command(about = "Periksa kesehatan server, Docker, dan konfigurasi")]
    Doctor,

    #[command(about = "Backup dan restore database aplikasi")]
    Db {
        #[command(subcommand)]
        action: DbCommands,
    },

    #[command(about = "Kelola aplikasi")]
    App {
        #[command(subcommand)]
        action: AppCommands,
    },

    #[command(name = "default-app", about = "Lihat atau setel aplikasi default")]
    DefaultApp {
        app: Option<String>,
    },

    #[command(about = "Kelola scoped agent token")]
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },

    #[command(name = "agent-guide", about = "Panduan operasional Skoci untuk AI agent")]
    AgentGuide,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SecretCommands {
    #[command(about = "Simpan secret untuk aplikasi")]
    Set {
        key: String,
        value: String,
    },
    #[command(about = "Daftar nama kunci secret aplikasi")]
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DbCommands {
    #[command(about = "Buat backup database")]
    Backup {
        #[arg(default_value = "db")]
        service: String,
    },
    #[command(about = "Daftar riwayat backup database")]
    List,
    #[command(about = "Restore database dari backup")]
    Restore {
        backup_id: String,
        #[arg(default_value = "db")]
        service: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    #[command(about = "Hapus aplikasi")]
    Delete {
        app: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TokenCommands {
    #[command(about = "Buat scoped agent token baru")]
    Create {
        #[arg(long)]
        app: String,
        #[arg(long, default_value = "agent")]
        name: String,
    },
    #[command(about = "Daftar token untuk aplikasi")]
    List {
        #[arg(long)]
        app: String,
    },
    #[command(about = "Cabut token")]
    Revoke {
        #[arg(long)]
        app: String,
        #[arg(long)]
        name: String,
    },
}
