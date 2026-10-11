use serde::Serialize;
use skoci_api::error::SkociError;
use skoci_api::models::ApiResponse;
use skoci_api::responses::{
    ApprovalInfo, DeploymentInfo, DoctorResponse, LogsResponse, StatusResponse, TokenInfo,
};

pub fn print_json<T: Serialize>(resp: &ApiResponse<T>) {
    match serde_json::to_string_pretty(resp) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Gagal memformat JSON: {e}"),
    }
}

pub fn print_error(err: &SkociError, is_json: bool) {
    if is_json {
        let resp = ApiResponse::<()>::err(err.clone());
        print_json(&resp);
    } else {
        eprintln!("✗ Error [{}]: {}", err.code, err.message);
        eprintln!("  Solusi: {}", err.fix);
    }
}

pub fn render_status(status: &StatusResponse) {
    println!("Aplikasi: {}", status.app);
    println!("Aplikasi Default: {}", if status.is_default_app { "Ya" } else { "Tidak" });

    if let Some(dep_id) = &status.running_deployment {
        println!("Deployment sedang berjalan: {dep_id}");
    }

    println!("\nLingkungan (Environments):");
    if status.environments.is_empty() {
        println!("  (Belum ada lingkungan yang di-deploy)");
    } else {
        println!("  {:<12} {:<18} {:<12} {:<8} URL", "ENV", "VERSI", "STATUS", "SEHAT");
        println!("  {}", "-".repeat(70));
        for env in &status.environments {
            let healthy_str = if env.healthy { "✓" } else { "✗" };
            let urls_str = if env.urls.is_empty() {
                "-".to_string()
            } else {
                env.urls.join(", ")
            };
            println!(
                "  {:<12} {:<18} {:<12} {:<8} {}",
                env.env, env.version, env.container_status, healthy_str, urls_str
            );
        }
    }

    if !status.services.is_empty() {
        println!("\nLayanan Terhubung (Services):");
        for (service, state) in &status.services {
            println!("  • {}: {}", service, state);
        }
    }
}

pub fn render_deployments(deployments: &[DeploymentInfo]) {
    if deployments.is_empty() {
        println!("Belum ada riwayat deployment.");
        return;
    }

    println!("{:<24} {:<16} {:<12} {:<18} {:<10} DIBUAT", "ID", "APP", "ENV", "VERSI", "STATUS");
    println!("{}", "-".repeat(95));
    for dep in deployments {
        let ts = dep.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        println!(
            "{:<24} {:<16} {:<12} {:<18} {:<10} {}",
            dep.id, dep.app, dep.env, dep.version, dep.status, ts
        );
    }
}

pub fn render_approvals(approvals: &[ApprovalInfo]) {
    if approvals.is_empty() {
        println!("Tidak ada permintaan persetujuan yang menunggu.");
        return;
    }

    println!("{:<24} {:<16} {:<12} {:<10} {:<18} DETAIL", "REQUEST ID", "APP", "AKSI", "STATUS", "PEMOHON");
    println!("{}", "-".repeat(95));
    for app in approvals {
        println!(
            "{:<24} {:<16} {:<12} {:<10} {:<18} {}",
            app.id, app.app, app.action, app.status, app.requested_by, app.detail
        );
    }
}

pub fn render_tokens(tokens: &[TokenInfo]) {
    if tokens.is_empty() {
        println!("Belum ada token untuk aplikasi ini.");
        return;
    }

    println!("{:<16} {:<16} {:<16} {:<20} TERAKHIR DIGUNAKAN", "ID (12 HEX)", "APP", "NAMA", "DIBUAT");
    println!("{}", "-".repeat(85));
    for tok in tokens {
        let created = tok.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let last_used = tok
            .last_used_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<16} {:<16} {:<16} {:<20} {}",
            tok.id, tok.app, tok.name, created, last_used
        );
    }
}

pub fn render_doctor(doctor: &DoctorResponse) {
    let health_str = if doctor.healthy { "Sehat (Healthy)" } else { "Bermasalah (Issues Detected)" };
    println!("Status Server : {}", health_str);
    println!("Versi Server  : {}", doctor.version);
    println!("Idle RAM      : {:.1} MB", doctor.idle_ram_mb);
    println!("\nHasil Pemeriksaan:");

    for check in &doctor.checks {
        let icon = if check.passed { "✓" } else { "✗" };
        println!("  {} {:<24} : {}", icon, check.name, check.message);
        if let Some(fix) = &check.fix {
            if !check.passed {
                println!("    Solusi: {}", fix);
            }
        }
    }
}

pub fn render_logs(logs: &LogsResponse) {
    println!("Log Aplikasi: {} ({}) Kontainer: {}", logs.app, logs.env, logs.container);
    println!("{}", "-".repeat(70));
    for line in &logs.lines {
        println!("{}", line);
    }

    if !logs.error_summary.is_empty() {
        println!("\nRingkasan Error ({} baris):", logs.error_summary.len());
        println!("{}", "-".repeat(70));
        for err_line in &logs.error_summary {
            println!("  [!] {}", err_line);
        }
    }
}
