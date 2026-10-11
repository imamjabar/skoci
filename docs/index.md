# Skoci — Dokumentasi Resmi

> Platform deployment self-hosted yang sangat ringan (<20 MB RAM), aman sejak desain, dan dirancang khusus untuk agen coding AI sebagai operator utama dengan dashboard validasi untuk manusia.

---

## 4 Keunikan Utama

1. **Sangat Ringan (<20 MB Idle RAM Target):**
   Satu binary Rust murni menyatukan control plane, embedded reverse proxy (port 80 dan 443), SQLite database, dan dashboard. Menghemat 95% RAM dibandingkan Coolify (~800 MB) dan Dokploy (~600 MB), sehingga VPS hemat $4/bulan atau Raspberry Pi tetap lega untuk aplikasi pengguna.

2. **Agen AI Sebagai Operator Utama:**
   CLI dan MCP (Model Context Protocol) dibuat dari satu sumber kebenaran API (`skoci-api`) dengan paritas 100%. Setiap output terstruktur JSON (`--json`), error menyertakan solusi perbaikan `{code, message, fix}`, dan skill bawaan tersedia untuk berbagai harness (OMP, Claude Code, Pi, Codex, OpenClaw).

3. **Dashboard untuk Validasi, Bukan Konfigurasi:**
   Semua konfigurasi aplikasi tinggal di repositori Git (`skoci.yaml`). Dashboard berfungsi sebagai Mission Control manusia: melihat status kesehatan kontainer, memeriksa bukti deploy (`skoci.evidence.json`), memberikan persetujuan promosi production, dan rollback 1-klik.

4. **Aman Sejak Desain:**
   - **Zero Shell Concatenation:** Docker dipanggil lewat argumen terstruktur (`tokio::process::Command`), kebal terhadap prompt injection.
   - **Zero SSH untuk Agen:** Agen hanya berkomunikasi melalui HTTPS REST API dan token scoped per aplikasi (`sk_agent_<app>_*`).
   - **Enkripsi Secret:** Secret dienkripsi saat tersimpan di node (ChaCha20-Poly1305), disamarkan di log, dan tidak pernah dikembalikan ke prompt LLM.
   - **Server-Side Build:** Laptop Anda tidak membutuhkan Docker; kompilasi image dilakukan sepenuhnya oleh server VPS.

---

## Tabel Perbandingan Platform

| Fitur | Coolify | Dokploy | Kamal | **Skoci** |
| :--- | :--- | :--- | :--- | :--- |
| **Arsitektur Binary** | Monolith PHP/Laravel (~800 MB RAM) | Monorepo Node/Next.js (~600 MB RAM) | Ruby Gem CLI (stateless) | **1 Binary Rust murni (<20 MB RAM)** |
| **Reverse Proxy** | Traefik container terpisah | Traefik container terpisah | `kamal-proxy` Go container | **Embedded Hyper/Tokio pada port 80 & 443** |
| **Database Internal** | PostgreSQL + Redis container | PostgreSQL container | Tidak ada | **SQLite tertanam (zero extra daemon)** |
| **Akses Server** | Port SSH root / web login | Port SSH root / web login | **Wajib Root SSH Keys** | **Zero SSH: Scoped HTTPS Token per App** |
| **Operator AI Agent** | MCP addon read-only | CLI terpisah (~8% API) | Tidak ada | **Native CLI & MCP (100% Parity)** |
| **Format Error** | Log error mentah | Exception stack trace | Log error | **`{code, message, fix}` terstruktur** |
| **Peran Dashboard** | Konfigurasi 100+ form input | Konfigurasi form input | Tidak ada | **Validasi, Evidence, & Approval** |
| **Dukungan Raspberry Pi** | Berat / sering OOM di Pi 4 2GB | Berat di 2GB | Ringan (tapi butuh SSH) | **Sangat Ringan (<20MB, native ARM64)** |

---

## Memulai Cepat (Quickstart 3 Menit)

### 1. Pasang Skoci di Server VPS / Raspberry Pi 64-bit
Jalankan di server:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```
Pastikan port **80** dan **443** terbuka pada firewall penyedia VPS Anda.

### 2. Buat Token Aplikasi di Server
Di server VPS, buat token khusus untuk aplikasi Anda:
```bash
skoci token create --app toko --name laptop
```

### 3. Hubungkan Laptop ke Server
Di laptop atau mesin lokal pengembang Anda:
```bash
skoci connect --server https://skoci.<base-domain> --token sk_agent_toko_xxxx
```

### 4. Deploy dari Repositori Proyek
Masuk ke direktori repositori aplikasi Anda:
```bash
cd ~/projects/toko
skoci init
skoci deploy --env preview
```

Aplikasi Anda langsung aktif di `https://toko-preview.<base-domain>`.

---

## Navigasi Dokumentasi

* [Persyaratan Server & Instalasi](installation.md)
* [Deploy Aplikasi Pertama](deploy-first-app.md)
* [Deploy Database & Backup](deploy-first-database.md)
* [Integrasi AI Agent (MCP & Skill)](agent-native.md)
* [Arsitektur Keamanan & Scoped Token](security.md)
