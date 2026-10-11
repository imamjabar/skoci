# ⛵ Skoci

> **Platform Deployment Agent-Native yang Sangat Ringan (<20 MB RAM)**  
> Agen AI yang men-deploy, manusia yang memvalidasi lewat dashboard, dan aplikasi tetap berjalan di server VPS / Raspberry Pi Anda sendiri.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![RAM Idle](https://img.shields.io/badge/RAM%20Idle-7.8%20MB-brightgreen.svg)](#arsitektur)
[![Binary Size](https://img.shields.io/badge/Binary%20Size-12%20MB-success.svg)](#arsitektur)
[![Language](https://img.shields.io/badge/Rust-2021-orange.svg)](#)

---

## Mengapa Skoci?

Alat deploy yang ada (**Coolify**, **Dokploy**) dirancang untuk diklik manusia lewat ratusan form web, memakan **600–800 MB RAM** saat idle, dan rentan terhadap command injection saat dioperasikan agen AI.  
Sedangkan **Kamal** hemat memori di server, tetapi mewajibkan akses root SSH yang berisiko tinggi dan buta tanpa dashboard.

**Skoci hadir mengisi posisi unik:**
* **Seringan Kamal** (target idle <20 MB, aktual **~7.8 MB**).
* **Seaman yang dibutuhkan saat agen memegang kendali** (zero shell concatenation, zero SSH keys, secret terenkripsi).
* **Dashboard rapi untuk validasi** (status kesehatan, bukti deploy `skoci.evidence.json`, persetujuan manusia, rollback 1-klik).

---

## Instalasi 1 Baris (VPS, Raspberry Pi 64-bit, Dedicated, Laptop)

```bash
curl -fsSL https://zen.ai.id/skoci/install.sh | bash
```

Dapat dijalankan di:
- **VPS / Cloud:** Hetzner, DigitalOcean, Linode, AWS Lightsail, OVHcloud, Hostinger ($4/bulan cukup!)
- **Raspberry Pi:** Raspberry Pi 4 atau 5 (wajib OS 64-bit: Raspberry Pi OS 64-bit atau Ubuntu Server 64-bit)
- **Dedicated Server / Homelab:** PC atau laptop bekas dengan Linux 64-bit.

---

## Alur Kerja Cepat (Quickstart)

### 1. Inisialisasi Proyek (`skoci init`)
Masuk ke repositori aplikasi Anda (Node, Python, Go, Rust, Laravel, atau Docker):
```bash
skoci init
```
→ Menghasilkan `skoci.yaml` dan `skoci.policy.yaml`.

### 2. Simpan Secret Terenkripsi
```bash
skoci secret set DATABASE_URL "postgresql://user:pass@localhost:5432/db"
```
*(Secret dienkripsi ChaCha20-Poly1305 di node; agen tidak bisa membaca kembali dan otomatis disamarkan di log).*

### 3. Deploy ke Preview
```bash
skoci deploy --env preview
```
→ Build kontainer → health check `/healthz` → zero-downtime cutover → catat `skoci.evidence.json`.

### 4. Promosi ke Production (Human Gate)
```bash
skoci promote "Semua tes lolos, siap rilis ke production"
```
→ Operator manusia menyetujui lewat Dashboard Web di `http://IP_SERVER:8787` atau CLI:
```bash
skoci --token sk_master_admin approve req_xxxx
```

### 5. Rollback 1-Klik
```bash
skoci rollback
```

---

## Tabel Perbandingan Platform

| Fitur | Coolify | Dokploy | Kamal | **Skoci** |
| :--- | :--- | :--- | :--- | :--- |
| **Arsitektur Binary** | Monolith PHP/Laravel (~800 MB RAM) | Monorepo Node/Next.js (~600 MB RAM) | Ruby Gem CLI (stateless) | **1 Binary Rust murni (~7.8 MB RAM)** |
| **Reverse Proxy** | Traefik container terpisah | Traefik container terpisah | `kamal-proxy` Go container | **Embedded Hyper/Tokio di dalam binary** |
| **Database Internal** | PostgreSQL + Redis container | PostgreSQL container | Tidak ada | **SQLite tertanam (zero extra daemon)** |
| **Akses Server** | Port SSH root / web login | Port SSH root / web login | **Wajib Root SSH Keys** | **Zero SSH: Scoped HTTPS Token per App** |
| **Operator AI Agent** | MCP addon read-only | CLI terpisah (~8% API) | Tidak ada | **Native CLI & MCP (100% Parity)** |
| **Format Error** | Log error mentah | Exception stack trace | Log error | **`{code, message, fix}` terstruktur** |
| **Peran Dashboard** | Konfigurasi 100+ form input | Konfigurasi form input | Tidak ada | **Validasi, Evidence, & Approval** |
| **Dukungan Raspberry Pi** | Berat / sering OOM di Pi 4 2GB | Berat di 2GB | Ringan (tapi butuh SSH) | **Sangat Ringan (<20MB, native ARM64)** |

---

## Dokumentasi Lengkap

* [Persyaratan Server & Instalasi](docs/installation.md)
* [Deploy Aplikasi Pertama](docs/deploy-first-app.md)
* [Deploy Database & Backup](docs/deploy-first-database.md)
* [Integrasi AI Agent (MCP & Skill)](docs/agent-native.md)
* [Arsitektur Keamanan & Scoped Token](docs/security.md)

---

## Lisensi

Didistribusikan di bawah lisensi Apache-2.0. Lihat [LICENSE](LICENSE) untuk informasi lebih lanjut.
