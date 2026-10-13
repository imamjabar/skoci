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
* **Gerbang Persetujuan Manusia:** Promosi produksi pertama sebuah aplikasi langsung aktif (karena belum ada trafik produksi yang berisiko); setiap promosi berikutnya, pemulihan database, dan penghapusan aplikasi memerlukan persetujuan manusia via web dashboard (`https://<server-ip>:8443`) atau `sudo skoci approve <id>` di server (`--strict-approvals` mewajibkan persetujuan untuk promosi pertama juga).
* **Enkripsi Secret:** Variabel rahasia dienkripsi saat tersimpan di host dengan ChaCha20-Poly1305, disuntikkan via berkas sementara, disamarkan di log, dan tidak pernah dikembalikan ke konteks agen.
* **HTTPS Otomatis & Verifikasi Domain:** Embedded reverse proxy menerbitkan sertifikat TLS Let's Encrypt. `skoci server setup` secara otomatis mengaktifkan magic DNS (`<dashed-ip>.sslip.io`) saat `--domain` tidak disertakan sehingga deploy preview langsung mendapatkan URL HTTPS aktif, sedangkan production wajib menggunakan domain kustom yang diverifikasi dengan `skoci domain check <host>` (kecuali operator mengaktifkan `--allow-magic-dns-production`).
* **Tiga Status Hasil Deploy:** Deployment menunggu kesiapan URL publik pada tahap smoke test lalu melaporkan status `live` (beroperasi normal), `degraded` (health check lulus, namun smoke test `GET /` gagal; periksa dengan `skoci logs`), atau `failed`. Rollback dengan satu perintah (`skoci rollback`).
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
Saat `--domain` tidak disertakan, `skoci server setup` otomatis mengaktifkan magic DNS (`<dashed-ip>.sslip.io`) dan menampilkan peringatan, sehingga deploy preview pertama Anda langsung mendapatkan URL HTTPS yang dapat diakses (`https://<app>-preview.<dashed-ip>.sslip.io`). Untuk memakai domain milik Anda sejak awal, berikan `--domain domainanda.com` (dan opsional `--email anda@domainanda.com`), atau mulai dengan magic DNS lalu beralih kapan saja memakai `skoci domain set domainanda.com --primary`. Aturan firewall UFW (mengizinkan port SSH, 80, 443, dan 8443 dengan kebijakan default deny incoming) dikonfigurasi secara default.

Flag setup yang berguna saat menyiapkan server baru:
* `--domain <domain>` dan `--email <email>`: menyetel base domain server (`<app>.<domain>`, `<app>-preview.<domain>`, dan dashboard `skoci.<domain>`) serta alamat email kontak Let's Encrypt sejak awal sebagai pengganti `sslip.io`.
* `--ssh-copy-id`: memasang public key SSH Anda pada VPS yang hanya mendukung autentikasi password (membuat `~/.ssh/id_ed25519` jika Anda belum memiliki key), alih-alih gagal.
* `--ssh-password-stdin`: membaca password SSH dan `sudo -S` jarak jauh dari baris pertama stdin untuk bootstrap non-interaktif (password tidak pernah diletakkan di `argv`, menggantikan skrip pembungkus `expect` manual).
* `--harden-ssh`: menulis `/etc/ssh/sshd_config.d/00-skoci-harden.conf` yang menonaktifkan autentikasi password dan keyboard-interactive serta menyetel `PermitRootLogin prohibit-password`. Berkas ini diurutkan sebelum drop-in bawaan distro seperti `50-cloud-init.conf`, karena `sshd` memakai nilai pertama yang dibacanya. Divalidasi dengan `sshd -t` lalu diperiksa ulang dengan `sshd -T` sebelum reload, dibatalkan jika salah satu gagal, dan dilewati jika akun tidak memiliki `authorized_keys`.
* `--no-ufw`: melewati konfigurasi firewall UFW (secara default, setup mengizinkan port SSH, 80, 443, dan 8443 lalu mengaktifkan `ufw` dengan default deny incoming; dilewati beserta petunjuk jika `ufw` belum terpasang).
* `--strict-approvals`: menulis `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` ke `/etc/skoci/skoci.env` sehingga setiap promosi produksi oleh scoped agent token wajib disetujui manusia, termasuk promosi pertama aplikasi.
* `--allow-magic-dns-production`: menulis `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true` ke `/etc/skoci/skoci.env` agar operator dapat mendemokan deployment production pada hostname `sslip.io` tanpa mengedit konfigurasi manual lewat SSH. Menampilkan peringatan saat setup karena hostname magic-DNS secara default ditolak di production (pemilik zona dapat menerbitkan sertifikat valid untuk hostname tersebut, dan semua pengguna berbagi kuota rate-limit Let's Encrypt).

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
`/etc/skoci` (`0700 root:root`) dan `/etc/skoci/skoci.env` (`0600 root:root`) hanya dapat dibaca oleh `root` dan `/etc/skoci/skoci.env` adalah satu-satunya tempat penyimpanan `SKOCI_MASTER_TOKEN`, sehingga perintah admin di server memerlukan `sudo`. Baca master token dengan `sudo grep '^SKOCI_MASTER_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2-`. `skoci server setup` membuatkan scoped token secara otomatis dan tidak pernah mencetaknya ke layar.

### 3. Hubungkan Laptop ke Server
Di komputer kerja Anda, pasang client dan hubungkan workspace Anda menggunakan IP server, token, dan pin sertifikat SPKI:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only
skoci connect --server <server-public-ip> --token <token> --pin sha256:<spki-pin>
```
Secara default, `skoci connect` hanya menyimpan profil di `~/.config/skoci/servers.json` (`0600`). Tambahkan `--project-env` jika ingin menulis `.skoci/env` (`0600`, otomatis ditambahkan ke `.gitignore`) untuk kebutuhan CI atau konfigurasi khusus per-repositori. Berkas `.skoci/env` tetap dibaca bila ada, dan berkas legacy `.env.skoci` dimigrasikan otomatis ke `.skoci/env`.

### 4. Inisialisasi & Deploy ke Preview
Masuk ke repositori proyek Anda:
```bash
skoci init
skoci validate --verbose
skoci deploy --env preview
```
`skoci init` mendeteksi manifest proyek dan menggunakan namanya sebagai nama default aplikasi (`name` di `package.json` setelah awalan `@scope/` dibuang, lalu `[package] name` di `Cargo.toml`, lalu nama di `pyproject.toml` `[project]`/`[tool.poetry]`), dan hanya beralih ke nama direktori jika manifest tidak ditemukan. Nama akhir divalidasi langsung di awal (1–40 karakter alfanumerik huruf kecil dan tanda hubung), sehingga nama yang tidak valid gagal seketika di `init` disertai panduan perbaikan alih-alih gagal saat deploy sebagai `scope_violation`.

`skoci init` menjaga root proyek tetap bersih — semua yang dihasilkannya berada di `.skoci/`:
```
your-project/
├── .gitignore        # bertambah satu baris: .skoci/env
├── your files …      # tidak disentuh
└── .skoci/
    ├── skoci.yaml    # konfigurasi aplikasi (commit ke git)
    ├── Dockerfile    # resep hasil generate, dilewati jika sudah ada (commit ke git)
    ├── ignore        # filter unggahan; .dockerignore dipakai jika berkas ini tidak ada (commit ke git)
    └── env           # opsional: hanya ditulis oleh 'skoci connect --project-env' (0600, tidak pernah diunggah)
```
Berkas legacy `skoci.yaml` dan `.env.skoci` di root proyek otomatis dimigrasikan ke `.skoci/skoci.yaml` dan `.skoci/env` oleh `skoci init`, `skoci validate`, `skoci deploy`, dan `skoci connect`; pemuat konfigurasi hanya membaca `.skoci/skoci.yaml`. Dockerfile yang digenerate menghapus direktori `.skoci/` dari container image akhir (termasuk `/usr/share/nginx/html/.skoci` pada template statis dan static-build) serta berjalan sebagai pengguna non-root (`node`, `appuser`, atau `nginx`). Template statis dan static-build menggunakan `nginxinc/nginx-unprivileged:1.27-alpine` pada port `8080` (`port: 8080` di `.skoci/skoci.yaml`) dan mengembalikan `404` untuk dotfile, ekstensi kode/konfigurasi/database/log internal (`*.md`, `*.py`, `*.sh`, `*.sql`, `*.yaml`, `*.toml`, `*.lock`, `*.log`, `.env`), serta manifest proyek (`package.json`, `yarn.lock`, `requirements.txt`, `Makefile`, `Dockerfile*`), sementara berkas data `*.json` tetap dapat disajikan.

`skoci validate [path]` (mendukung `--json`) memvalidasi `.skoci/skoci.yaml` secara offline tanpa panggilan jaringan dan tanpa melakukan deployment, memeriksa jalur konfigurasi, nama aplikasi, Dockerfile/konteks, domain produksi dan preview yang dideklarasikan, serta secret yang dibutuhkan (keluar dengan kode 0 jika valid, 1 jika tidak valid disertai `{code, message, fix}`). Gunakan `--verbose` (`-v`) untuk menampilkan daftar berkas persis yang akan diunggah oleh `skoci deploy`, total ukuran byte, dan jalur sensitif atau terabaikan yang dikecualikan. `skoci deploy` menerapkan filter unggahan secara eksplisit dan berurutan: aturan bawaan (beserta deny list berkas sensitif yang tidak dapat dikesampingkan), `.gitignore` di root proyek, lalu `.skoci/ignore` (atau `.dockerignore` saat `.skoci/ignore` tidak ada). Berkas `.gitignore` di direktori induk, gitignore global, dan `.git/info/exclude` tidak diterapkan.

`skoci deploy` menjalankan alur deployment atomik: build, peluncuran kontainer, health check, cutover, dan smoke test root (`GET /`). Jika smoke test gagal pada aplikasi yang sudah memiliki route aktif, Skoci secara otomatis mengembalikan rute ke kontainer sebelumnya di proxy router dan SQLite, menghentikan dan menghapus kontainer baru yang rusak, dan menyelesaikan status sebagai `degraded` yang mengarahkan ke `skoci logs <app>` — versi yang sedang melayani trafik tetap aktif. Jika terdeteksi port yang tidak dapat digunakan atau health check timeout, guard pembersihan segera menghentikan dan menghapus kontainer yang gagal beserta image tag yang tidak lagi dirujuk tanpa meninggalkan kontainer yatim; pembersihan image juga dijalankan setelah deploy gagal. Konkurensi dijaga ketat per aplikasi: operasi deploy, promote, rollback, dan delete pada aplikasi yang sama bersifat mutually exclusive; permintaan yang bersamaan mengembalikan error `conflict` disertai ID deployment yang sedang berjalan.

### 5. Promosikan ke Production & Setujui
Secara default, production membutuhkan domain milik Anda. Arahkan `A` record ke server, tambahkan ke `.skoci/skoci.yaml` pada bagian `domains:`, verifikasi, lalu ajukan promosi:
```bash
skoci domain check app.yourdomain.com
skoci promote "Semua pengujian preview lolos, siap rilis produksi"
```
Saat dijalankan menggunakan scoped agent token (`sk_agent_*`):
* **Promosi pertama:** Jika aplikasi belum memiliki deployment production aktif, promosi pertama langsung berhasil dan aktif melayani trafik (tidak ada trafik produksi berjalan yang berisiko).
* **Promosi berikutnya:** Setiap promosi selanjutnya berhenti di kode keluar 2, berstatus `pending_approval`, dan mencetak `Request ID: appr_…`. Agen tidak dapat menyetujui permintaannya sendiri dan harus menyampaikan request ID tersebut kepada operator manusia, yang menyetujuinya melalui web dashboard di `https://<server-public-ip>:8443` (atau `https://skoci.<base-domain>`) atau di terminal server:
  ```bash
  sudo skoci approve <request_id>
  ```
* **Persetujuan ketat (`--strict-approvals`):** Jika server disiapkan dengan `skoci server setup --strict-approvals` (atau `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` di `/etc/skoci/skoci.env`), promosi pertama juga wajib disetujui manusia.
* **Master token:** Menjalankan perintah di server menggunakan master token akan mempromosikan langsung tanpa permintaan persetujuan.
* **Kebijakan persetujuan rollback:** Scoped agent token dapat me-rollback production ke versi yang aktif tepat sebelumnya (last known-good) tanpa persetujuan. Me-rollback ke versi historis yang lebih lama menghasilkan permintaan persetujuan (`appr_…`, kode keluar 2, aksi `rollback`). Jika `--strict-approvals` aktif, seluruh rollback production memerlukan persetujuan manusia. Rollback preview tidak dibatasi.

**Alur Preview → Production tanpa domain kustom (`sslip.io`):** Secara default, `skoci promote` menolak hostname magic-DNS (`sslip.io`, `nip.io`) di production karena pemilik zona pihak ketiga dapat menerbitkan sertifikat valid untuk hostname Anda dan semua pengguna berbagi kuota rate-limit Let's Encrypt. Untuk mencoba alur lengkap dari awal hingga akhir tanpa domain:
1. Aktifkan override di sisi server saat setup (`skoci server setup --host <server-public-ip> --user root --allow-magic-dns-production`) atau setel `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true` di `/etc/skoci/skoci.env` lalu jalankan `sudo systemctl restart skoci` (pengaturan boolean di `/etc/skoci/skoci.env` menerima `true/false`, `1/0`, `yes/no`, dan `on/off`, tidak sensitif huruf besar/kecil).
2. Deploy ke preview: `skoci deploy --env preview` langsung menyajikan aplikasi di `https://<app>-preview.<dashed-ip>.sslip.io`.
3. Promosikan ke production: `skoci promote "Rilis perdana"` langsung aktif di `https://<app>.<dashed-ip>.sslip.io` pada promosi pertama, sedangkan promosi berikutnya mengembalikan `Request ID: appr_…` (kode keluar 2) untuk disetujui lewat `sudo skoci approve <request_id>`.

### 6. Beralih dari sslip.io ke Domain Kustom
Setelah Anda membeli domain, konfigurasikan sebagai base domain utama server Anda:
```bash
skoci domain check mydomain.com
skoci domain set mydomain.com --primary
```
Perintah kedua menyimpan base domain, mengarahkan ulang setiap rute aktif ke `<app>.mydomain.com` dan `<app>-preview.mydomain.com`, memindahkan dashboard ke `skoci.mydomain.com`, dan memicu penerbitan sertifikat untuk hostname baru tanpa me-restart server atau memutus permintaan. Menambahkan `--force` akan melewati pra-pemeriksaan DNS. Pastikan `A` record untuk `mydomain.com` dan `skoci.mydomain.com` sudah mengarah ke server terlebih dahulu.

### Dua Endpoint: Control Plane vs. Dashboard
Skoci menyediakan dua endpoint HTTPS terpisah untuk kebutuhan yang berbeda:

| Endpoint | Default | Sertifikat | Untuk |
| --- | --- | --- | --- |
| `https://<server-ip>:8443` | selalu aktif | Sertifikat ACME untuk IP, atau self-signed sebelum terbit | Agen dan CLI. Kepercayaan berasal dari SPKI pin, jadi peringatan browser di sini memang diharapkan dan bukan cacat. |
| `https://skoci.<domain-anda>` | hanya setelah base domain disetel | Let's Encrypt | Manusia lewat browser, tanpa peringatan |

### Membangun Ulang Server di IP yang Sama
VPS yang dipasang ulang memiliki host key SSH baru dan kunci TLS Skoci baru, sehingga kedua jangkar kepercayaan yang Anda simpan menjadi usang. Setup menolak melanjutkan dan menyebutkan dengan tepat mana yang berubah:
```bash
ssh-keygen -R <server-ip>                                   # host key SSH berubah
skoci server setup --host <server-ip> --user root           # menyimpan SPKI pin dan token baru
```
Jika perintah biasa gagal dengan `pin_mismatch`, berarti kunci TLS server yang berubah: jalankan setup lagi, atau hubungkan ulang memakai pin dari `sudo skoci server fingerprint`. Kedua galat ini tidak pernah diterima otomatis — gejala yang sama muncul saat terjadi penyadapan.

---

## Berikan ke Agen Anda

Salin dan tempelkan instruksi prompt ini ke asisten coding AI Anda (Claude Code, OMP, Cursor, dll.):

```text
Operational Rules:
1. Selalu jalankan perintah dengan `--json` untuk output terstruktur (contoh: `skoci deploy --json`).
2. Jika perintah gagal, periksa `error.code` dan jalankan perintah yang disarankan pada `error.fix`.
3. Kode keluar 2 berarti membutuhkan persetujuan manusia (promosi produksi lanjutan, atau promosi pertama bila `--strict-approvals` aktif, serta restore db dan hapus app). Laporkan `request_id` kepada pengguna agar dapat disetujui di dashboard pada https://<server-ip>:8443 atau via `sudo skoci approve <request_id>`.
4. Jangan pernah mencoba atau meminta akses SSH. Semua interaksi berjalan via HTTPS ter-pinning menggunakan scoped token.
5. Deployment produksi wajib menggunakan domain kustom asli (magic DNS *.sslip.io ditolak). Verifikasi DNS sebelum deploy dengan `skoci domain check <host>`.
6. Perlakukan deployment `degraded` sebagai kegagalan yang perlu diselidiki: periksa log dengan `skoci logs --tail 100 --json` dan jangan promosikan ke produksi.
7. Jangan pernah mencetak, mencatat di log, atau meng-commit token (`sk_agent_*`, `sk_master_*`).
```

---

## Keamanan

* **Bebas Injeksi Shell:** Perintah Docker dan sistem dijalankan sebagai vektor argumen terstruktur (`tokio::process::Command`); input dari user atau agen tidak pernah disisipkan ke string shell.
* **Scoped Token Terisolasi & Input via Stdin:** Token agen (`sk_agent_<app>_*`) disimpan sebagai hash SHA-256; batasan aplikasi ditegakkan di SQLite. Token dan secret dapat dialirkan via stdin untuk mencegah kebocoran pada daftar proses (`ps`, `/proc`) atau riwayat shell.
* **Kebijakan Tanpa SSH & SPKI Pinning:** Tidak ada private key SSH yang disimpan pada workstation pengembang atau lingkungan agen. Semua komunikasi diarahkan via `https://<server-ip>:8443` yang dipin ke kunci SPKI permanen server (`sha256:...`).
* **Enkripsi Secret, Proteksi Kunci & Retensi:** Variabel rahasia dan kredensial database accessory dienkripsi dengan ChaCha20-Poly1305 pada node menggunakan `/var/lib/skoci/node.key` (mode `0600`). Daemon menolak berjalan jika `node.key` hilang atau tidak terbaca saat secret ada di SQLite, mencegah hilangnya data rahasia secara permanen. Operator wajib mencadangkan `/var/lib/skoci/node.key` (`SKOCI_ALLOW_SECRET_KEY_RESET=true` adalah override operator untuk membuat ulang kunci dan menghapus baris yang tidak dapat dibaca). Berkas env sementara `0600` langsung dihapus setelah kontainer berjalan, dan log secara otomatis menyamarkan nilai secret, token Skoci (`sk_agent_…`, `sk_master_…`), serta header `Authorization: Bearer …` sebagai `[REDACTED]`. Kebijakan retensi memangkas deployment historis (default 30 per aplikasi, 90 hari), persetujuan yang telah diputuskan (30 hari), dan log audit (90 hari) tanpa menyentuh route aktif atau persetujuan tertunda. Kontainer berjalan dengan opsi log terbatas (`--log-opt max-size=10m --log-opt max-file=3`), dan cache BuildKit dibersihkan setiap jam. Kredensial client di `.skoci/env` dibatasi pada allowlist (`SKOCI_SERVER_URL`, `SKOCI_TOKEN`, `SKOCI_SERVER_PIN`), dan subprocess dijalankan dengan environment yang bersih.
* **Filter Unggahan Eksplisit & Deny List Pengemasan:** `skoci deploy` menerapkan aturan bawaan, deny list berkas sensitif yang tidak dapat dikesampingkan (`.env*`, `*.pem`, `*.key`, `id_*`, `.npmrc`, `.aws/`, `.ssh/`), `.gitignore` di root proyek, dan `.skoci/ignore` (atau `.dockerignore`), tanpa mewarisi gitignore direktori induk atau global. Periksa daftar unggahan secara rinci dengan `skoci validate --verbose`.
* **Isolasi Kontrol Lokal & Pinned TLS Bind:** API hanya mengikat `127.0.0.1:8787` secara lokal dan `0.0.0.0:8443` dengan TLS ter-pinning SPKI. Pada listener HTTPS publik (port 443), header `:authority`/`Host` wajib cocok dengan TLS SNI (mencegah domain fronting dengan HTTP `421 Misdirected Request`).
* **Proteksi Kepemilikan Host:** Domain kustom dan auto-host dipartisi secara ketat; klaim silang atau pembajakan host antar-aplikasi ditolak.
* **Gerbang Persetujuan Manusia & Pinning ID Image:** Promosi produksi pertama sebuah aplikasi langsung berhasil secara default (`--strict-approvals` / `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` mewajibkan persetujuan untuk promosi pertama juga); setiap promosi berikutnya, pemulihan database, dan operasi destruktif mewajibkan persetujuan manusia yang dikunci pada image ID Docker yang tidak dapat diubah (mencegah manipulasi TOCTOU).
* **Hardening Systemd:** Layanan server diamankan dengan `UMask=0077`, `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `PrivateDevices=true`, `ProtectKernelTunables=true`, `ProtectKernelModules=true`, `ProtectControlGroups=true`, `RestrictSUIDSGID=true`, `LockPersonality=true`, `RestrictNamespaces=true`, dan `LimitNOFILE=65536`.
* **Log Audit, Pengujian CI & Provenance Build:** Setiap operasi mutasi dicatat pada log audit append-only. CI menjalankan pengujian integrasi Docker end-to-end (`scripts/e2e_docker.sh`) yang memverifikasi startup daemon, perutean fixture, aturan deny, eksekusi pengguna non-root, sanitasi `.skoci`, dan penghapusan kontainer yang bersih saat deploy ulang. Binary rilis resmi dilengkapi atestasi provenance GitHub Actions (`gh attestation verify`).

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

### Ke 0.4.3
`.skoci/env` tetap dibaca untuk `SKOCI_SERVER_URL`, `SKOCI_TOKEN`, dan `SKOCI_SERVER_PIN`, tetapi ketiganya tidak lagi diekspor ke environment proses. Skrip yang mengandalkan `skoci` untuk meneruskan ketiga variabel itu ke proses anaknya harus membaca berkas tersebut sendiri. Variabel lain di `.skoci/env` tetap diekspor seperti sebelumnya.

### Ke 0.4.5
Dockerfile yang dihasilkan berubah: kini menghapus `.skoci/` dari image yang dibangun dan berjalan sebagai pengguna non-root bila base image memungkinkan. Proyek yang diinisialisasi sebelumnya tetap memakai Dockerfile lamanya, jadi tidak ada yang berubah sampai Anda mengadopsi templat baru:
```bash
rm .skoci/Dockerfile && skoci init && skoci deploy
```
Dockerfile yang Anda tulis sendiri tetap aman; `skoci init` tidak pernah menimpa berkas yang sudah ada.

### Ke 0.6.1
* **Pelaporan rollback lebih jelas:** Deploy yang gagal pada smoke test kini mencetak `⚠ Rolled back: version <baru> failed its smoke test; <sebelumnya> is serving again`, dan evidence deployment memuat `reverted_to_version`. Versi 0.6.0 sudah mengembalikan rute dengan benar, tetapi masih mencetak `Live but failing` yang menyebut URL yang sebenarnya sudah dilayani versi sebelumnya.
* **Pesan kehilangan kunci menyebut path sebenarnya:** Penolakan menjalankan daemon tanpa `node.key` kini mencetak path kunci yang sesungguhnya dari direktori data server, bukan `/var/lib/skoci/node.key` yang ditulis tetap.

### Ke 0.6.0
* **Cadangkan `/var/lib/skoci/node.key` sekarang:** Kunci enkripsi node mengenkripsi setiap secret dan kata sandi database accessory yang digenerate. Jika `node.key` hilang atau tidak terbaca saat secret ada di database, daemon menolak berjalan untuk mencegah hilangnya secret secara diam-diam. Cadangkan `/var/lib/skoci/node.key` segera (mode `0600`, pemilik `skoci:skoci`). Jika secret yang tersimpan hilang permanen dan Anda bersedia menghapus data yang tidak dapat dipulihkan, jalankan sekali dengan `SKOCI_ALLOW_SECRET_KEY_RESET=true` di `/etc/skoci/skoci.env`.
* **Pemangkasan retensi otomatis saat restart pertama:** Restart daemon pertama setelah upgrade langsung menjalankan pemeliharaan retensi otomatis, memangkas riwayat deployment lama (default 30 per aplikasi, 90 hari), persetujuan yang telah diputuskan (30 hari), dan log audit (90 hari). Baris yang masih dirujuk oleh route aktif atau persetujuan tertunda tidak akan pernah dihapus.
* **Deployment degraded otomatis di-rollback:** Deployment yang lolos health check internal tetapi gagal pada smoke test root (`GET /`) kini secara otomatis mengembalikan rute proxy dan database ke kontainer sebelumnya, menghapus kontainer baru yang rusak, dan menyelesaikan status sebagai `degraded` yang mengarahkan ke `skoci logs <app>`. Versi aplikasi yang sebelumnya berfungsi tetap melayani lalu lintas pengguna.
* **Kebijakan persetujuan rollback production:** Token agen scoped dapat me-rollback production ke versi yang aktif tepat sebelumnya (last known-good) tanpa persetujuan manusia. Me-rollback ke versi historis yang lebih lama kini menghasilkan permintaan persetujuan (`appr_…`, kode keluar 2, aksi `rollback`) yang harus disetujui manusia via `sudo skoci approve <id>`. Jika `--strict-approvals` aktif, seluruh rollback production memerlukan persetujuan.

### Ke 0.5.1
* **Perbaikan `--ssh-password-stdin`:** Bootstrap dengan kata sandi bisa gagal dengan pesan `ssh-copy-id failed` sebelum kunci terpasang, karena keluaran `ssh-copy-id` dibuang saat proses tersebut masih menulis. Perbarui klien (`curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only`) sebelum melakukan bootstrap ke server yang hanya menerima kata sandi.
* **Anggota grup `skoci` yang tertinggal:** Proses instalasi maupun upgrade kini mengeluarkan seluruh akun manusia dari grup `skoci`, membatalkan keanggotaan era pra-0.5.0 yang membuat master token dapat dibaca tanpa `sudo`. Tidak ada langkah manual yang diperlukan.

### Ke 0.5.0
* **Kontainer Static & StaticBuild:** Dockerfile situs statis hasil generate kini menggunakan `nginxinc/nginx-unprivileged:1.27-alpine` (`USER nginx`), mendengarkan pada port **8080**, menghapus `.skoci/` dari web root, serta mengembalikan `404` untuk dotfile, ekstensi kode/konfigurasi/log/database internal, dan manifest proyek. Buat ulang resep kontainer (`rm .skoci/Dockerfile && skoci init`) dan pastikan `.skoci/skoci.yaml` menyetel `port: 8080` di bawah `run:` sebelum melakukan deploy ulang.
* **Commit `.skoci/ignore` ke git:** Simpan `.skoci/ignore` (bersama `.skoci/skoci.yaml` dan `.skoci/Dockerfile`) di dalam git. Menghapus direktori `.skoci/` akan menghilangkan aturan pengecualian unggahan yang Anda tulis dan berisiko mempublikasikan berkas internal.
* **Perubahan flag firewall:** Konfigurasi firewall UFW kini aktif secara default pada `skoci server setup` (flag opt-in UFW versi lama telah dihapus); gunakan `--no-ufw` untuk melewati konfigurasi UFW.
* **Izin berkas server & `sudo`:** Direktori `/etc/skoci` (`0700 root:root`) dan `/etc/skoci/skoci.env` (`0600 root:root`) kini khusus `root`, berkas token terpisah warisan versi lama di `/etc/skoci/` dihapus saat upgrade, `/var/lib/skoci` berizin `0700 skoci:skoci`, dan pengguna SSH tidak lagi dimasukkan ke grup `skoci`. Jalankan perintah admin di server menggunakan `sudo` (`sudo skoci approve <id>`, `sudo skoci token create`, dll.), dan baca master token dengan `sudo grep '^SKOCI_MASTER_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2-`.
* **Aturan abaikan unggahan yang eksplisit:** `skoci deploy` kini hanya menerapkan aturan bawaan, `.gitignore` di root proyek, dan `.skoci/ignore` (atau `.dockerignore`). Berkas `.gitignore` di direktori induk, gitignore global, dan `.git/info/exclude` tidak lagi diterapkan secara diam-diam — pindahkan aturan yang masih diperlukan ke `.skoci/ignore`.
* **Audit unggahan & migrasi tata letak otomatis:** Berkas legacy `skoci.yaml` dan `.env.skoci` di root otomatis dimigrasikan ke `.skoci/skoci.yaml` dan `.skoci/env`, serta `skoci connect` kini hanya menulis `~/.config/skoci/servers.json` kecuali diberi flag `--project-env`. Jalankan `skoci validate --verbose` satu kali setelah upgrade untuk memeriksa daftar berkas yang akan diunggah oleh `skoci deploy`.

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
