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
* **Streaming Progres & Durasi Per-Tahap:** Log build real-time, durasi tiap tahap, progress bar dengan estimasi waktu selesai, dan event pengalihan proxy.
* **Control Plane TLS Ter-Pinning & Scoped Token:** Control plane terhubung via HTTPS di port `8443` ter-pinning ke kunci publik SPKI permanen (`sha256:...`). Token per-aplikasi (`sk_agent_<app>_*`) menggantikan kunci SSH root.
* **Gerbang Persetujuan Manusia:** Promosi produksi, pemulihan database, dan penghapusan aplikasi memerlukan persetujuan manusia via web dashboard (`https://<server-ip>:8443`) atau CLI master.
* **Enkripsi Secret:** Variabel rahasia dienkripsi saat tersimpan di host dengan ChaCha20-Poly1305, disuntikkan via berkas sementara, disamarkan di log, dan tidak pernah dikembalikan ke konteks agen.
* **HTTPS Otomatis & Verifikasi Domain:** Embedded reverse proxy menerbitkan sertifikat TLS Let's Encrypt. Pra-validasi record DNS sebelum sertifikat diterbitkan dengan `skoci domain check <host>`. Aplikasi produksi wajib menggunakan domain kustom (magic DNS `*.sslip.io` bersifat opt-in via `--magic-dns` dan ditolak di production karena risiko kontrol zona pihak ketiga dan kuota rate-limit bersama).
* **Tiga Status Hasil Deploy:** Verifikasi rute root via smoke test: `live` (beroperasi normal), `degraded` (health check lulus, namun smoke test `GET /` gagal; periksa dengan `skoci logs`), atau `failed`. Rollback dengan satu perintah (`skoci rollback`).
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
       │    (kemas repo, patuhi .skoci/ignore)    │
       ├─────────────────────────────────────────►│
       │    Unggah HTTPS dengan scoped token      │ 2. Server kompilasi image Docker
       │                                          │ 3. Jalankan container & health check
       │                                          │ 4. Pengalihan atomik proxy (port 80/443)
       │ 5. Aplikasi aktif + URL publik           │
       │◄─────────────────────────────────────────┤
```

---

## Panduan Cepat

### 1. Pasang atau Setup VPS
Konfigurasi VPS jarak jauh langsung dari laptop Anda melalui SSH (Skoci >= 0.4.0):
```bash
skoci server setup --host <server-public-ip> --user root
```
Atau pasang langsung di terminal server Linux 64-bit Anda (pastikan port **80**, **443**, dan **8443** terbuka di firewall cloud):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### 2. Buat Scoped Token dan Baca Pin (di VPS)
Jika memasang manual di server, buat token agen yang dibatasi hak aksesnya khusus untuk aplikasi Anda, lalu baca pin kunci server:
```bash
sudo skoci token create --app my-app --name laptop
sudo skoci server fingerprint
```
Kedua perintah berjalan tanpa flag tambahan di server: CLI membaca `/etc/skoci/skoci.env` dan menargetkan control plane lokal. `skoci server setup` melakukannya untuk Anda dan tidak pernah mencetak token.

### 3. Hubungkan Laptop ke Server
Di komputer kerja Anda, pasang client dan hubungkan workspace Anda menggunakan IP server, token, dan pin sertifikat SPKI:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only
skoci connect --server <server-public-ip> --token <token> --pin sha256:<spki-pin>
```
Profil disimpan di `~/.config/skoci/servers.json` (0600) dan `.skoci/env` (0600, masuk `.gitignore`). Berkas legacy `.env.skoci` tetap dibaca untuk proyek sebelum 0.4.1.

### 4. Inisialisasi & Deploy ke Preview
Masuk ke repositori proyek Anda:
```bash
skoci init
skoci deploy --env preview
```
`skoci init` menjaga root proyek tetap bersih — semua yang dihasilkannya berada di `.skoci/`:
```
your-project/
├── .gitignore        # bertambah satu baris: .skoci/env
├── your files …      # tidak disentuh
└── .skoci/
    ├── skoci.yaml    # konfigurasi aplikasi
    ├── Dockerfile    # resep hasil generate (dilewati jika Anda sudah punya sendiri)
    ├── ignore        # filter unggahan; .dockerignore Anda dipakai jika berkas ini tidak ada
    └── env           # URL server, token, pin (0600, tidak pernah diunggah)
```
`skoci.yaml` atau `Dockerfile` yang ditulis oleh versi lama, atau oleh Anda sendiri, tetap dibaca dari root proyek.

### 5. Promosikan ke Production & Setujui
Production membutuhkan domain milik Anda. Arahkan A record ke server, tambahkan ke `.skoci/skoci.yaml` pada bagian `domains:`, verifikasi, lalu ajukan promosi:
```bash
skoci domain check app.yourdomain.com
skoci promote "Semua pengujian preview lolos, siap rilis produksi"
```
Dijalankan dengan scoped agent token, perintah berhenti di kode keluar 2 dan mencetak `request_id`; promosi menunggu persetujuan manusia. Setujui melalui web dashboard di `https://<server-public-ip>:8443` atau lewat terminal server:
```bash
sudo skoci approve <request_id>
```
Master token di server berperan sebagai manusia: promosi langsung berjalan tanpa permintaan persetujuan.

---

## Berikan ke Agen Anda

Salin dan tempelkan instruksi prompt ini ke asisten coding AI Anda (Claude Code, OMP, Cursor, dll.):

```text
Operational Rules:
1. Selalu jalankan perintah dengan `--json` untuk output terstruktur (contoh: `skoci deploy --json`).
2. Jika perintah gagal, periksa `error.code` dan jalankan perintah yang disarankan pada `error.fix`.
3. Kode keluar 2 berarti membutuhkan persetujuan manusia (deploy produksi, restore db, hapus app). Laporkan `request_id` kepada pengguna agar dapat disetujui di dashboard pada https://<server-ip>:8443 atau via `skoci approve <request_id>`.
4. Jangan pernah mencoba atau meminta akses SSH. Semua interaksi berjalan via HTTPS ter-pinning menggunakan scoped token.
5. Deployment produksi wajib menggunakan domain kustom asli (magic DNS *.sslip.io ditolak). Verifikasi DNS sebelum deploy dengan `skoci domain check <host>`.
6. Perlakukan deployment `degraded` sebagai kegagalan yang perlu diselidiki: periksa log dengan `skoci logs --tail 100 --json` dan jangan promosikan ke produksi.
7. Jangan pernah mencetak, mencatat di log, atau meng-commit token (`sk_agent_*`, `sk_master_*`).

---

## Keamanan

* **Bebas Injeksi Shell:** Perintah Docker dan sistem dijalankan sebagai vektor argumen terstruktur (`tokio::process::Command`); input dari user atau agen tidak pernah disisipkan ke string shell.
* **Scoped Token Terisolasi & Input via Stdin:** Token agen (`sk_agent_<app>_*`) disimpan sebagai hash SHA-256; batasan aplikasi ditegakkan di SQLite. Token dan secret dapat dialirkan via stdin untuk mencegah kebocoran pada daftar proses (`ps`, `/proc`) atau riwayat shell.
* **Kebijakan Tanpa SSH & SPKI Pinning:** Tidak ada private key SSH yang disimpan pada workstation pengembang atau lingkungan agen. Semua komunikasi diarahkan via `https://<server-ip>:8443` yang dipin ke kunci SPKI permanen server (`sha256:...`).
* **Enkripsi Secret & Sanitasi Environment:** Variabel rahasia dienkripsi dengan ChaCha20-Poly1305 pada node, diinjeksi melalui berkas sementara `0600` yang langsung dihapus, dan disamarkan sebagai `[REDACTED]` di log. Kredensial client di `.skoci/env` (atau legacy `.env.skoci`) dibatasi pada allowlist (`SKOCI_SERVER_URL`, `SKOCI_TOKEN`), dan subprocess dijalankan dengan environment yang bersih.
* **Deny List Pengemasan:** Perintah deploy secara otomatis mengecualikan berkas sensitif (`.env*`, `*.pem`, `*.key`, `id_*`, `.npmrc`, `.aws/`, `.ssh/`) dan mematuhi `.skoci/ignore` (atau `.dockerignore`), serta kompresi arsip menggunakan izin privat `0600`.
* **Isolasi Kontrol Lokal & Pinned TLS Bind:** API hanya mengikat `127.0.0.1:8787` secara lokal dan `0.0.0.0:8443` dengan TLS ter-pinning SPKI. Pada listener HTTPS publik (port 443), header `:authority`/`Host` wajib cocok dengan TLS SNI (mencegah domain fronting dengan HTTP `421 Misdirected Request`).
* **Proteksi Kepemilikan Host:** Domain kustom dan auto-host dipartisi secara ketat; klaim silang atau pembajakan host antar-aplikasi ditolak.
* **Gerbang Persetujuan Manusia & Pinning ID Image:** Rilis produksi, pemulihan database, dan operasi destruktif mewajibkan persetujuan manusia yang dikunci pada image ID Docker yang tidak dapat diubah (mencegah manipulasi TOCTOU).
* **Hardening Systemd:** Layanan server diamankan dengan `UMask=0077`, `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `PrivateDevices=true`, `ProtectKernelTunables=true`, `ProtectKernelModules=true`, `ProtectControlGroups=true`, `RestrictSUIDSGID=true`, `LockPersonality=true`, `RestrictNamespaces=true`, dan `LimitNOFILE=65536`.
* **Log Audit & Provenance Build:** Setiap operasi mutasi dicatat pada log audit append-only. Binary rilis resmi dilengkapi atestasi provenance GitHub Actions (`gh attestation verify`).

### Risiko Residual

* **Grup Docker ≈ Root:** Akses ke `/var/run/docker.sock` setara dengan hak akses root di Linux; Skoci memperketat proses server, namun akses soket Docker tetap berprivilese setara root.
* **Jaringan Outbound Kontainer Terbuka:** Kontainer mengeksekusi kode user/agen di dalam sandbox Docker standar dengan koneksi keluar internet yang tidak dibatasi.
* **Trust-on-First-Use (TOFU) via SSH:** Setup awal server (`skoci server setup`, Skoci >= 0.4.0) mem-bootstrap VPS dan menetapkan pin SPKI melalui SSH. Jika koneksi SSH pertama disadap tanpa verifikasi host key SSH, trust awal bisa terganggu. Koneksi berikutnya dipin secara permanen di `~/.config/skoci/servers.json`.
* **Isolasi Docker Server Tunggal:** Isolasi mengandalkan namespace dan cgroup kernel Linux bawaan Docker tanpa virtualisasi microVM perangkat keras.
* **Klaim Domain Kustom First-Come:** Agen dapat mendaftarkan domain kustom apa pun yang belum digunakan oleh aplikasi lain pada server tersebut.
* **Checksum Rilis dari Asal yang Sama:** Berkas checksum rilis berada pada rilis GitHub yang sama dengan binary; verifikasi keaslian dan integritas menggunakan `gh attestation verify skoci-linux-x86_64 --repo imamjabar/skoci`.

---

## Peningkatan Versi

### Ke 0.4.1 dan setelahnya
Upgrade server dan semua workstation secara bersamaan; client menolak berkomunikasi dengan server yang berbeda versi minor.
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```
Proyek yang sudah ada tetap berjalan tanpa perubahan: `skoci.yaml`, `Dockerfile`, dan `.env.skoci` di root tetap dibaca. Untuk memakai tata letak `.skoci/` pada sebuah proyek, hapus berkas hasil generate di root lalu jalankan `skoci init` lagi — pertahankan `Dockerfile` yang Anda tulis sendiri, berkas itu tetap berada di root pada kedua kasus.

### Dari 0.1

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
4. **Hubungkan Ulang & Perbarui Konfigurasi:** Hubungkan kembali workstation (`skoci connect`) dan set ulang secret (`skoci secret set`). Perbarui `.skoci/skoci.yaml` (atau legacy `skoci.yaml`): `env.public` kini berupa map (`KEY: value`), bidang `build.mode` dan `server` telah dihapus, dan `skoci.policy.yaml` dapat dihapus.

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
