# Deploy Aplikasi Pertama Anda di Skoci

Panduan langkah demi langkah mendeploy aplikasi web ke server Anda menggunakan Skoci.

---

## 1. Hubungkan Workspace ke Server Skoci

Sebelum mendeploy, hubungkan repositori lokal Anda ke server Skoci menggunakan token aplikasi yang telah dibuat di server:

```bash
skoci connect --server https://skoci.203-0-113-10.sslip.io --token sk_agent_toko_xxxxxxxxxxxx
```

Perintah ini akan memvalidasi koneksi ke server dan menyimpan kredensial secara aman di `.env.skoci` (izin berkas `0600`, otomatis ditambahkan ke `.gitignore`).

---

## 2. Inisialisasi Proyek (`skoci init`)

Jalankan perintah inisialisasi di repositori aplikasi Anda:

```bash
cd ~/projects/toko
skoci init
```

Skoci otomatis mendeteksi framework aplikasi Anda:
* **HTML statis:** ada `index.html` → Nginx alpine, port 80, endpoint `/healthz` otomatis.
* **Node.js dengan script `start`:** `package.json` → Dockerfile `node:22-alpine`, port 3000.
* **Frontend build (Vite/React tanpa `start`):** build dengan Node lalu disajikan Nginx, port 80.
* **Python / FastAPI / Flask / Django:** `requirements.txt` atau `pyproject.toml` → port 8000.
* **Go:** `go.mod` → binary statis `CGO_ENABLED=0`, port 8080.
* **Rust:** `Cargo.toml` → multi-stage cargo build, port 8080.
* **PHP / Laravel:** `composer.json` → `php:8.3-apache`, port 80.
* **Docker:** `Dockerfile` yang sudah ada dipakai apa adanya.

Database hanya ditambahkan ke `services` bila dependensinya terdeteksi (misal `pg`, `psycopg`, `mysql2`, `redis`). `skoci init` juga mencetak catatan (misal path health check yang dipilih) — baca sebelum deploy.

Hasil inisialisasi membuat:
1. `skoci.yaml`: File deklaratif kontrak aplikasi.
2. `Dockerfile`: Resep pembuatan image kontainer (jika belum ada).
3. `.dockerignore`: Menjamin direktori sensitif (`.git`, `.env.skoci`) tidak terunggah.

### Mengapa Laptop Anda Tidak Perlu Docker?
Dockerfile adalah resep instruksi. Kompilasi kontainer sepenuhnya dijalankan di server VPS Anda. Komputer lokal atau sandbox agen AI hanya mengemas berkas proyek dan mengirimkannya ke server.

---

## 3. Menyesuaikan `skoci.yaml`

File `skoci.yaml` menggunakan skema terstruktur:

```yaml
version: 1
app: toko
build:
  dockerfile: Dockerfile
  context: .
run:
  port: 3000                  # Port internal yang didengar aplikasi Anda
  health: /healthz            # Path HTTP health probe untuk zero-downtime swap
  health_timeout_secs: 60     # Batas waktu tunggu aplikasi siap (detik)
  memory: 256m                # Batas memori kontainer (mencegah OOM)
domains:
  - toko.sumopod.id           # Domain kustom untuk production
preview_domains:
  - toko-preview.sumopod.id   # Domain kustom untuk preview
env:
  public:
    NODE_ENV: production
  secrets:
    - API_KEY                 # Hanya nama kuncinya, bukan nilainya!
```

---

## 4. Menyimpan Secret Terenkripsi (`skoci secret set`)

Skoci memisahkan deklarasi nama variabel dengan nilai sensitifnya:
* Di `skoci.yaml`: Anda hanya mendaftarkan nama kunci (`API_KEY`).
* Nilai sebenarnya disimpan langsung ke server Skoci dalam bentuk terenkripsi:

```bash
skoci secret set API_KEY "kunci-rahasia-produksi"
```

Secret dienkripsi dengan **ChaCha20-Poly1305** di server memakai kunci node (`/var/lib/skoci/node.key`, dibuat otomatis saat instalasi). Nilai secret **tidak pernah dikembalikan ke agen AI**, disuntikkan ke container lewat env-file sementara (mode 0600, langsung dihapus), dan disamarkan di log.

---

## 5. Melakukan Deployment ke Preview

Deploy aplikasi Anda ke lingkungan preview:

```bash
skoci deploy --env preview
```

### Tahapan Alur Deploy Skoci:
1. **Packaging & Upload:** Berkas proyek dikemas menghormati `.gitignore` dan diunggah ke server.
2. **Extract:** Server mengekstrak arsip dan memvalidasi `skoci.yaml`.
3. **Build:** Server mengompilasi image Docker `skoci/<app>:<version>` dengan log live.
4. **Services:** Server memastikan kontainer database (jika ada) aktif dan siap menerima koneksi.
5. **Start:** Kontainer versi baru dijalankan dengan variabel rahasia yang disuntikkan secara aman via temporary env-file.
6. **Health Check:** Reverse proxy memantau `run.health` hingga kontainer merespons status HTTP 2xx/3xx.
7. **Cutover:** Reverse proxy menukar target rute kontainer secara atomik tanpa downtime.
8. **Smoke Test:** Pengujian HTTP GET dikirimkan langsung ke kontainer baru untuk memastikan respon sukses.
9. **Cleanup:** Kontainer versi lama dimatikan setelah masa drain 5 detik.

Aplikasi preview Anda langsung dapat diakses di:
`https://toko-preview.203-0-113-10.sslip.io` (atau preview domain kustom Anda).

*Catatan: membuka IP server langsung (`http://203.0.113.10`) melayani **aplikasi default** (aplikasi pertama yang di-deploy; ubah dengan `skoci default-app <app>` memakai master token). Alamat ini ikut dicetak di hasil deploy bila aplikasi Anda adalah aplikasi default.*

---

## 6. Menggunakan Custom Domain

Untuk menggunakan domain sendiri:
1. Arahkan DNS **A Record** dari domain Anda ke IP publik server:
   - `toko.sumopod.id` → `IP_SERVER`
   - `toko-preview.sumopod.id` → `IP_SERVER`
2. Daftarkan di `skoci.yaml`:
   ```yaml
   domains:
     - toko.sumopod.id
   preview_domains:
     - toko-preview.sumopod.id
   ```
3. Jalankan `skoci deploy --env preview` atau `skoci deploy --env production`. Reverse proxy Skoci otomatis menerbitkan sertifikat TLS resmi dari Let's Encrypt!

---

## 7. Memeriksa Status & Log

Periksa kondisi kontainer secara real-time:
```bash
skoci status
```

Periksa log keluaran kontainer (secret disamarkan sebagai `[REDACTED]`):
```bash
skoci logs --tail 50
```

---

## 8. Promosi ke Production (Human Gate)

Secara default, peluncuran ke production memerlukan izin operator manusia:

```bash
skoci promote "Semua pengujian lolos pada preview, siap rilis ke production"
```

Perintah mengembalikan exit code 2 (*menunggu persetujuan*) beserta `request_id`, misal `appr_4e052b4e...`.

Operator manusia dapat menyetujuinya lewat:
* **Web Dashboard:** Buka `https://skoci.<base-domain>`, masuk dengan master token, lalu klik tombol **Setujui**.
* **CLI Server:**
  ```bash
  sudo skoci approve appr_4e052b4e...
  ```

Setelah disetujui, kontainer production langsung aktif dan trafik dialihkan ke `https://toko.<base-domain>`.

---

## 9. Rollback 1-Klik

Jika versi baru bermasalah, kembalikan ke versi sebelumnya:

```bash
skoci rollback                 # versi sebelumnya di preview
skoci rollback --env production --to v20261011031108
```

Skoci menjalankan ulang image versi lama (tanpa build), menunggu health check lolos, lalu memindahkan trafik. Versi yang sedang live tetap melayani sampai versi lama dinyatakan sehat.
