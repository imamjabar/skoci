[English](README.md) | Bahasa Indonesia

# ⛵ Skoci

> **Biarkan agen coding AI Anda melakukan deploy ke VPS sendiri — dengan aman.**  
> Skoci adalah platform deployment agent-native yang sangat ringan, dibangun khusus untuk agen seperti Claude Code, OMP, dan Cursor. Skoci mengompilasi dan menjalankan kontainer di server Anda dengan HTTPS otomatis, token terisolasi, dan gerbang persetujuan manusia — dengan penggunaan memori idle hanya ~9 MB RSS.

```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

---

## Fitur Unggulan

* **Deploy Agen 1-Perintah:** Deploy langsung dari terminal agen dengan `skoci deploy`. Tanpa membutuhkan akses SSH, dan tanpa Docker di laptop — server yang mengompilasi image.
* **Streaming Progres Langsung:** Log build real-time, health check, dan event pengalihan rute proxy. Situs statis ter-deploy dalam ~10–20 detik setelah build pertama.
* **Scoped Token, Bukan Root SSH:** Token per-aplikasi (`sk_agent_<app>_*`) menggantikan kunci SSH root. Token diverifikasi via hash SHA-256 di SQLite dan terisolasi khusus untuk aplikasinya.
* **Gerbang Persetujuan Manusia:** Promosi produksi, pemulihan database, dan penghapusan aplikasi memerlukan persetujuan manusia via web dashboard atau CLI master.
* **Enkripsi Secret:** Variabel rahasia dienkripsi saat tersimpan di host dengan ChaCha20-Poly1305, disuntikkan via berkas sementara, disamarkan di log, dan tidak pernah dikembalikan ke konteks agen.
* **HTTPS Otomatis:** Embedded reverse proxy menerbitkan sertifikat TLS Let's Encrypt secara otomatis dengan domain gratis `*.sslip.io` atau domain kustom Anda.
* **Pengalihan Nol-Downtime & Rollback:** Pengalihan rute proxy atomik setelah lolos health check, dengan fitur rollback 1-perintah (`skoci rollback`) jika terjadi kendala.
* **Database Bawaan Lengkap:** Layanan deklaratif Postgres, MySQL, dan Valkey dengan backup terjadwal (cron), snapshot instan (`skoci db backup`), dan restore yang wajib disetujui manusia.
* **Agen yang Memperbaiki Diri:** Output JSON terstruktur (`--json`), rekomendasi `fix` yang jelas, dan kode keluar konsisten memungkinkan agen mendiagnosis dan memperbaiki error secara mandiri.
* **Binary Sangat Ringan:** Satu binary Rust mandiri dengan konsumsi ~9 MB RSS saat idle (diukur pada build rilis Linux), menyisakan seluruh ruang memori VPS untuk aplikasi Anda.

---

## Cocok Untuk / Tidak Cocok Untuk

### Cocok Untuk
* Solo builder dan vibe coder yang merilis proyek sampingan dan MVP menggunakan agen coding seperti Claude Code, OMP, atau Cursor.
* Tim kecil yang menjalankan beberapa aplikasi pada satu VPS terjangkau (Sumopod, Hetzner, DigitalOcean, atau Raspberry Pi di rumah).
* Siapa pun yang ingin agen AI dapat mendeploy dan memantau aplikasi tanpa menyerahkan akses root SSH.

### Tidak Cocok Untuk
* Kluster multi-server atau orkestrasi skala Kubernetes terdistribusi.
* Tim yang membutuhkan build cache lintas mesin atau registry kontainer eksternal terkelola.
* Beban kerja non-HTTP (misalnya protokol UDP/TCP mentah atau komputasi batch HPC).

---

## Cara Kerja

```
┌──────────────┐          ┌───────────────────────────────────────────────┐
│   Agen AI    │          │              Server Skoci (VPS)               │
│ (Claude/OMP) │          │                                               │
└──────┬───────┘          └───────────────────────┬───────────────────────┘
       │                                          │
       │ 1. skoci deploy                          │
       │    (kemas repo, patuhi .gitignore)       │
       ├─────────────────────────────────────────►│
       │    Unggah HTTPS dengan scoped token      │ 2. Server kompilasi image Docker
       │                                          │ 3. Jalankan container & health check
       │                                          │ 4. Pengalihan atomik proxy (port 80/443)
       │ 5. Aplikasi aktif + URL publik           │
       │◄─────────────────────────────────────────┤
```

---

## Panduan Cepat

### 1. Pasang di VPS
Jalankan di server Linux 64-bit Anda (pastikan port **80** dan **443** terbuka di firewall cloud):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### 2. Buat Scoped Token (di VPS)
Buat token agen yang dibatasi hak aksesnya khusus untuk aplikasi Anda:
```bash
sudo skoci token create --app my-app --name laptop
```

### 3. Hubungkan Laptop ke Server
Di komputer kerja Anda, pasang client dan hubungkan workspace Anda (macOS otomatis hanya memasang client; di laptop Linux tambahkan `--client-only`):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only
skoci connect --server https://skoci.<base-domain> --token <token>
```

### 4. Inisialisasi & Deploy ke Preview
Masuk ke repositori proyek Anda:
```bash
skoci init
skoci deploy --env preview
```

### 5. Promosikan ke Production & Setujui
Ajukan promosi ke production:
```bash
skoci promote "Semua pengujian preview lolos, siap rilis produksi"
```
Perintah mengembalikan kode keluar 2 dan `request_id`. Setujui melalui web dashboard di `https://skoci.<base-domain>` atau lewat terminal server:
```bash
sudo skoci approve <request_id>
```

---

## Berikan ke Agen Anda

Salin dan tempelkan instruksi prompt ini ke asisten coding AI Anda (Claude Code, OMP, Cursor, dll.):

```text
Anda dapat mendeploy dan mengelola aplikasi ini menggunakan Skoci CLI.
Aturan Operasional:
1. Selalu jalankan perintah dengan `--json` untuk output terstruktur (contoh: `skoci deploy --json`).
2. Jika perintah gagal, periksa `error.code` dan jalankan perintah yang disarankan pada `error.fix`.
3. Kode keluar 2 berarti membutuhkan persetujuan manusia (deploy produksi, restore db, hapus app). Laporkan `request_id` kepada pengguna agar dapat disetujui di dashboard atau via `skoci approve`.
4. Jangan pernah mencoba atau meminta akses SSH. Semua interaksi berjalan via HTTPS menggunakan scoped token.
5. Jangan pernah mencetak, mencatat di log, atau meng-commit token (`sk_agent_*`, `sk_master_*`).
```

---

## Keamanan

* **Bebas Injeksi Shell:** Perintah Docker dan sistem dijalankan sebagai vektor argumen (`tokio::process::Command`); input dari user atau agen tidak pernah disisipkan ke string shell.
* **Scoped Token Terisolasi:** Token agen (`sk_agent_<app>_*`) disimpan sebagai hash SHA-256; batasan aplikasi diverifikasi di SQLite.
* **Kebijakan Tanpa SSH:** Tidak ada private key SSH yang disimpan pada workstation pengembang atau lingkungan agen.
* **Enkripsi Secret:** Variabel rahasia dienkripsi dengan ChaCha20-Poly1305 pada node, diinjeksi melalui berkas sementara `0600`, dan disamarkan sebagai `[REDACTED]` di log.
* **Isolasi Kontrol Lokal:** Control plane mengikat interface `127.0.0.1:8787`, hanya dapat diakses melalui reverse proxy TLS atau tunnel SSH.
* **Gerbang Persetujuan Manusia:** Rilis produksi, pemulihan database, dan penghapusan aplikasi mewajibkan persetujuan manusia.
* **Log Audit:** Setiap operasi yang mengubah state dicatat beserta aktornya (`admin` atau `agent:<app>/<token>#<id>`); token mentah tidak pernah disimpan.

---

## Migrasi dari Versi 0.1

Bagi pengguna yang telah memasang Skoci versi 0.1:
1. **Bersihkan Konflik Port 80/443:** Hentikan kontainer manual lama yang mengikat port web:
   ```bash
   sudo docker ps --filter publish=80
   sudo docker rm -f <nama_kontainer>
   ```
2. **Jalankan Ulang Installer:** Perbarui binary dan layanan systemd di server:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
   ```
3. **Buat Ulang Scoped Token:** Token agen versi 0.1 dicabut otomatis saat startup karena pernah terekspos di endpoint audit lama. Buat token baru:
   ```bash
   sudo skoci token create --app <app> --name laptop
   ```
4. **Hubungkan Ulang & Perbarui Konfigurasi:** Hubungkan kembali workstation (`skoci connect`) dan set ulang secret (`skoci secret set`). Perbarui `skoci.yaml`: `env.public` kini berupa map (`KEY: value`), bidang `build.mode` dan `server` telah dihapus, dan `skoci.policy.yaml` dapat dihapus.

---

## Dokumentasi

* [Persyaratan Server & Panduan Instalasi](docs/installation.md)
* [Panduan Deploy Aplikasi Pertama](docs/deploy-first-app.md)
* [Panduan Deploy Database & Manajemen Backup](docs/deploy-first-database.md)
* [Arsitektur Agent-Native & Integrasi MCP](docs/agent-native.md)
* [Model Keamanan & Arsitektur Pertahanan](docs/security.md)

---

## Lisensi

Didistribusikan di bawah Lisensi Apache-2.0. Lihat [LICENSE](LICENSE) untuk informasi lebih lanjut.
