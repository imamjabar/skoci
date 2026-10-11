# ⛵ Skoci

> **Platform Deployment Agent-Native yang Sangat Ringan (<20 MB RAM)**  
> Agen AI yang men-deploy dari laptop Anda, server VPS yang mengompilasi dan menjalankan kontainer, reverse proxy otomatis dengan TLS, dan manusia yang memvalidasi lewat dashboard.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![RAM Budget](https://img.shields.io/badge/RAM%20Budget-%3C20%20MB-brightgreen.svg)](#arsitektur)
[![Binary Size](https://img.shields.io/badge/Binary%20Size-%3C20%20MB-success.svg)](#arsitektur)
[![Language](https://img.shields.io/badge/Rust-2021-orange.svg)](#)

---

## Mengapa Skoci?

Platform PaaS konvensional (**Coolify**, **Dokploy**) dirancang untuk diklik manusia lewat puluhan form web dan memakan **600–800 MB RAM** saat idle. Sebaliknya, **Kamal** hemat memori, namun mewajibkan akses root SSH yang berisiko tinggi dan buta tanpa dashboard validasi.

**Skoci mengisi posisi yang unik:**
* **Sangat Ramping:** Target penggunaan RAM idle **<20 MB**, satu binary Rust mandiri tanpa daemon database tambahan (SQLite tertanam).
* **Dirancang untuk Agen Coding AI:** Argumen aman berbasis vektor argv tanpa interpolasi shell string, token scoped per aplikasi (`sk_agent_<app>_*`), dan format respons terstruktur `{code, message, fix}`.
* **Server-Side Build:** Laptop Anda tidak membutuhkan Docker! Laptop hanya mengemas kode sumber; server VPS yang mengompilasi image Docker.
* **Trafik Publik Otomatis:** Reverse proxy bawaan mengikat port 80 dan 443 dengan sertifikat TLS otomatis via ACME (Let's Encrypt).
* **Human-in-the-Loop:** Aksi produksi dan pemulihan database memerlukan persetujuan manusia melalui Web Dashboard atau CLI.

---

## Alur Kerja Lengkap (End-to-End Workflow)

### 1. Pasang Skoci di Server VPS

Jalankan perintah instalasi 1-baris pada server Linux 64-bit (VPS Sumopod, Hetzner, DigitalOcean, atau Raspberry Pi 64-bit):

```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

Installer otomatis:
- Menyiapkan Docker Engine (jika belum ada).
- Menyiapkan pengguna sistem terisolasi `skoci` dan unit systemd.
- Mengonfigurasi master token di `/etc/skoci/master_token`.
- Mengatur base domain otomatis via sslip.io (misal `203-0-113-10.sslip.io`, dari IP publik server) atau domain Anda sendiri (`--domain contoh.id`, butuh DNS wildcard `*.contoh.id` → IP server).
- Jika sertifikat HTTPS belum bisa terbit (firewall, limit Let's Encrypt untuk sslip.io), installer memberi tahu dan menampilkan cara akses lewat SSH tunnel.
- Mengaktifkan sertifikat TLS otomatis via ACME pada port 80 dan 443.

> **PENTING:** Buka port **80** dan **443** pada firewall cloud penyedia VPS Anda (misal dashboard firewall Sumopod, Hetzner, atau AWS Security Group).

---

### 2. Buat Token Aplikasi di Server

Di terminal server VPS Anda, buat token akses untuk aplikasi Anda (misal aplikasi bernama `toko`):

```bash
skoci token create --app toko --name laptop
```

Perintah ini menghasilkan token berformat `sk_agent_toko_<hex>`. Token ini hanya memiliki hak akses ke aplikasi `toko` dan tidak bisa menyentuh aplikasi lain.

---

### 3. Hubungkan Laptop ke Server (`skoci connect`)

Di laptop pengembang Anda, pasang binary Skoci client:
```bash
# macOS
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

Lalu hubungkan workspace laptop ke server VPS Anda:
```bash
skoci connect --server https://skoci.203-0-113-10.sslip.io --token sk_agent_toko_xxxxxxxxxxxx
```
*Perintah ini memvalidasi koneksi dengan `skoci doctor` dan menyimpan kredensial ke `.env.skoci` (izin 0600, otomatis ditambahkan ke `.gitignore`).*

---

### 4. Inisialisasi Proyek (`skoci init`)

Masuk ke direktori proyek aplikasi Anda (Node.js, Python, Go, Rust, PHP, atau proyek ber-Dockerfile):

```bash
cd ~/projects/toko
skoci init
```

Skoci otomatis mendeteksi stack aplikasi dan membuat:
1. `skoci.yaml`: Konfigurasi deklaratif aplikasi.
2. `Dockerfile`: Resep build kontainer (jika belum ada).
3. `.dockerignore`: Menjaga file sensitif agar tidak terunggah.

> **Catatan Dockerfile:** Dockerfile adalah resep instruksi. Laptop Anda **tidak perlu menjalankan Docker**. Proses kompilasi kontainer sepenuhnya dijalankan oleh server VPS Skoci.

Contoh file `skoci.yaml`:
```yaml
version: 1
app: toko
build:
  dockerfile: Dockerfile
  context: .
run:
  port: 3000
  health: /healthz
  health_timeout_secs: 60
  memory: 256m
domains:
  - toko.sumopod.id
preview_domains:
  - toko-preview.sumopod.id
env:
  public:
    NODE_ENV: production
  secrets:
    - API_KEY
services:
  db:
    engine: postgres
    version: "16"
    backup:
      schedule: "0 2 * * *"
      keep: 7
```

Simpan nilai secret (dienkripsi ChaCha20-Poly1305 di server):
```bash
skoci secret set API_KEY "kunci-rahasia-anda"
```

---

### 5. Deploy ke Preview (`skoci deploy`)

Kirim aplikasi Anda ke server:

```bash
skoci deploy --env preview
```

Output menampilkan progres langsung (*live progress*), contoh nyata:
```text
▸ Deploy toko → preview
✓ Upload 1.4 MB (141 file) 0.1s
  Versi: v20261011041957  ID: dep_4cb242ea…
  ✓ [1/8] Ekstrak kode
  ⠋ [2/8] Build image 12s
    │ #8 [4/7] RUN npm install          ← 5 baris log build terakhir, diperbarui di tempat
  ✓ [3/8] Siapkan database
  ✓ [4/8] Jalankan container
  ✓ [5/8] Health check
  ✓ [6/8] Alihkan trafik
  ✓ [7/8] Smoke test
  ✓ [8/8] Bersihkan versi lama

✓ Live: https://toko-preview.203-0-113-10.sslip.io
✓ Live: http://203.0.113.10          ← hanya jika toko adalah aplikasi default
Versi: v20261011041957  Durasi: 21.3s
```
Jika gagal, CLI mencetak 30 baris log terakhir, pesan error, `Solusi:`, dan perintah `skoci watch <id>`. Versi lama tetap melayani trafik.

---

### 6. Promosi ke Production (Human Approval Gate)

Setelah versi preview divalidasi, ajukan promosi ke production:

```bash
skoci promote "Semua tes preview lolos, siap rilis ke production"
```

Skoci mengembalikan exit code 2 (*menunggu persetujuan*) beserta `request_id`, misal `appr_4e052b4e...`.

Operator manusia dapat menyetujuinya lewat:
1. **Web Dashboard:** Masuk ke `https://skoci.<base-domain>` dan klik **Setujui Promosi**.
2. **Terminal Server (Master):**
   ```bash
   sudo skoci approve appr_4e052b4e...
   ```

Setelah disetujui, kontainer production langsung aktif di:
`https://toko.<base-domain>` (atau domain kustom Anda).

---

## Menggunakan Domain Sendiri (Custom Domain)

Jika Anda memiliki domain sendiri (misal `sumopod.id`):
1. Buat **DNS A Record** di penyedia DNS Anda yang mengarah ke IP publik VPS:
   - `toko.sumopod.id` → `IP_VPS`
   - `toko-preview.sumopod.id` → `IP_VPS`
   - `skoci.sumopod.id` → `IP_VPS` (untuk dashboard)
2. Masukkan nama domain ke `skoci.yaml`:
   ```yaml
   domains:
     - toko.sumopod.id
   preview_domains:
     - toko-preview.sumopod.id
   ```
3. Jalankan `skoci deploy`. Reverse proxy Skoci otomatis menerbitkan sertifikat HTTPS resmi Let's Encrypt via TLS-ALPN-01 tanpa konfigurasi tambahan!

---

## Mengakses Dashboard Validasi

Dashboard web Skoci tidak memuat data di awal dan berkomunikasi langsung dengan control plane:
- **Akses HTTPS:** Buka `https://skoci.<base-domain>` di browser Anda. Masuk menggunakan Master Token yang tersimpan di server (`sudo cat /etc/skoci/master_token`).
- **Akses SSH Tunnel (jika TLS off):**
  ```bash
  ssh -L 8787:127.0.0.1:8787 user@IP_VPS
  ```
  Lalu buka `http://localhost:8787` di browser lokal Anda.

Control plane Skoci mengikat `127.0.0.1:8787` secara lokal, sehingga tidak pernah terekspos secara mentah ke internet publik.

---

## Standar Kode Keluar (Exit Codes)

Skoci mengikuti standar kode keluar yang dapat dipahami skrip otomatisasi dan agen coding:

| Kode | Kategori | Keterangan |
| :---: | :--- | :--- |
| `0` | **Sukses** | Perintah berhasil dieksekusi sampai tuntas. |
| `1` | **Galat Operasional** | Kegagalan build, health check timeout, atau konfigurasi tidak valid. |
| `2` | **Persetujuan Diperlukan** | Aksi membutuhkan persetujuan manusia (`promote`, `db restore`, `app delete`). |
| `3` | **Galat Keamanan** | Kegagalan autentikasi (`unauthorized`, `scope_violation`, `rate_limited`). |

---

## Tabel Solusi Masalah (Troubleshooting)

| Gejala Masalah | Penyebab Umum | Solusi yang Disarankan |
| :--- | :--- | :--- |
| **Port 80/443 Sibuk saat Startup** | Ada web server lama (Nginx/Apache) atau kontainer Docker lain yang mengikat port 80. | Periksa proses dengan `sudo ss -ltnp 'sport = :80'`. Jika disebabkan kontainer Docker lain, periksa `docker ps --filter publish=80` lalu hentikan dengan `docker rm -f <nama>`. |
| **Health Check Timeout (85%)** | Kontainer gagal merespons path `run.health` (misal `/healthz`) dalam batas waktu `run.health_timeout_secs`. | Skoci otomatis menyertakan 40 baris log kontainer terakhir. Periksa apakah port aplikasi di Dockerfile cocok dengan `run.port` di `skoci.yaml`. |
| **Build Gagal di Server (35%)** | Sintaks Dockerfile keliru atau dependensi package gagal terunduh. | Periksa log keluaran build yang ditampilkan real-time. Jalankan `skoci logs` untuk detail tambahan. |
| **Aplikasi Tidak Dapat Diakses dari Luar** | Firewall penyedia cloud memblokir trafik port 80 atau 443. | Buka port 80 dan 443 di panel firewall penyedia cloud (Sumopod, Hetzner, AWS, atau DigitalOcean). |
| **Pemeriksaan Diagnostik Sistem** | Ingin memverifikasi kesehatan daemon Docker, sertifikat TLS, dan database SQLite. | Jalankan `skoci doctor` baik dari laptop maupun dari server. |

---

## Model Keamanan & Pertahanan (Security Model)

1. **Scoped Token Terisolasi:** Token agen (`sk_agent_<app>_*`) diverifikasi langsung menggunakan hash SHA-256 di database SQLite. Token aplikasi `toko` tidak dapat membaca log atau mengubah secret aplikasi `blog`.
2. **Pencabutan Token Instan:** Admin dapat mencabut token kapan saja:
   ```bash
   skoci token revoke --app toko --name laptop
   ```
3. **Session Cookie Dashboard Aman:** Dashboard menggunakan session cookie `skoci_session` (HttpOnly, SameSite=Strict) dan mewajibkan header CSRF `X-Skoci-Dashboard: 1` untuk semua request mutasi.
4. **Isolasi Bind Lokal:** Control API mengikat `127.0.0.1:8787`. Akses publik wajib melalui domain API reverse proxy ber-TLS atau SSH tunnel.
5. **Pengerasan Systemd & Docker Group:** Service dijalankan dengan `ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, dan kapabilitas `CAP_NET_BIND_SERVICE`.  
   *(Catatan keamanan: pengguna di dalam grup `docker` memiliki akses setara root pada host; batasi akses login ke server VPS).*
6. **Enkripsi Secret:** Secret dienkripsi menggunakan ChaCha20-Poly1305 dengan kunci node acak `/var/lib/skoci/node.key` (0600). Saat kontainer dijalankan, nilai disuntikkan via file environment sementara berizin 0600 yang langsung dihapus dari disk setelah `docker run`.
7. **Audit Log Bersih:** Log audit mencatat aktor dan aksi, namun tidak pernah menyimpan plain-text token.
8. **Batas Ukuran Unggahan:** Unggahan arsip dibatasi secara default sebesar 200 MB (`--max-upload-mb`) untuk melindungi server dari serangan resource exhaustion.

---

## Catatan Migrasi untuk Instalasi Lama

Bagi pengguna yang telah memasang Skoci 0.1:
1. Hentikan kontainer yang dijalankan manual di port 80/443 (proxy Skoci sekarang memakai port itu):
   ```bash
   sudo docker ps --filter publish=80
   sudo docker rm -f <nama_kontainer_lama>
   ```
2. Jalankan kembali installer di server untuk memperbarui binary dan unit systemd:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
   ```
3. Saat pertama start, Skoci menghapus data palsu versi lama (apps/deployments/approvals), secret lama, dan **mencabut semua token agen lama** (token itu pernah terekspos lewat endpoint audit publik). Buat token baru:
   ```bash
   sudo skoci token create --app <nama_app> --name laptop
   ```
4. Di laptop: `skoci connect --server <url> --token <token-baru>`, set ulang secret (`skoci secret set`), lalu sesuaikan `skoci.yaml`: `env.public` sekarang berupa map `NAMA: nilai` (hapus daftar lama seperti `- PORT`), `build.mode`/`server` tidak dipakai lagi, dan file `skoci.policy.yaml` boleh dihapus.

---

## Dokumentasi Mendalam

* [Persyaratan Server & Instalasi Detail](docs/installation.md)
* [Panduan Deploy Aplikasi Pertama](docs/deploy-first-app.md)
* [Panduan Deploy Database & Backup](docs/deploy-first-database.md)
* [Arsitektur Agent-Native & Integrasi MCP](docs/agent-native.md)
* [Model Keamanan Lengkap](docs/security.md)

---

## Lisensi

Didistribusikan di bawah lisensi Apache-2.0. Lihat [LICENSE](LICENSE) untuk informasi lebih lanjut.
