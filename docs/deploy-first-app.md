# Deploy Aplikasi Pertama Anda di Skoci

Panduan langkah demi langkah mendeploy aplikasi web ke server Anda menggunakan Skoci.

---

## 1. Masuk ke Repositori Proyek Anda

Buka terminal di komputer lokal atau lingkungan agen Anda:

```bash
cd ~/my-awesome-app
```

Pastikan Anda memiliki kredensial koneksi ke server Skoci Anda:
```bash
export SKOCI_SERVER_URL="http://IP_SERVER_ANDA:8787"
export SKOCI_TOKEN="sk_agent_my-awesome-app_..."
```

---

## 2. Inisialisasi Proyek (`skoci init`)

Jalankan perintah inisialisasi:
```bash
skoci init
```

Skoci otomatis mendeteksi framework Anda:
* **Node.js / Next.js:** mendeteksi `package.json` → membuat Dockerfile multi-stage alpine ramping, port 3000.
* **Python / FastAPI / Django:** mendeteksi `requirements.txt` atau `pyproject.toml` → port 8000.
* **Go:** mendeteksi `go.mod` → kompilasi binary statis CGO_ENABLED=0 di container alpine, port 8080.
* **Rust:** mendeteksi `Cargo.toml` → multi-stage cargo build, port 8080.
* **Laravel / PHP:** mendeteksi `composer.json` → PHP 8.3 cli alpine, port 80.
* **Docker Generic:** menggunakan Dockerfile yang sudah ada.

Hasil inisialisasi menghasilkan 2 file kontrak:
1. `skoci.yaml` (konfigurasi aplikasi)
2. `skoci.policy.yaml` (kebijakan hak akses agen)

---

## 3. Menyesuaikan `skoci.yaml`

File `skoci.yaml` deklaratif dan mudah dibaca manusia maupun agen:

```yaml
version: 1
app: toko
server: default
build:
  mode: server          # server (default) | local
  dockerfile: Dockerfile
run:
  port: 3000            # Port internal container Anda
  health: /healthz      # Path health check untuk zero-downtime swap
  memory: 256m          # Batas memori container (mencegah OOM)
domains:
  - toko.example.com
env:
  public:
    - NODE_ENV
    - PORT
  secrets:
    - DATABASE_URL      # Tulis NAMA saja, bukan nilai rahasianya!
    - API_SECRET_KEY
```

---

## 4. Menyimpan Secret Terenkripsi (`skoci secret set`)

Skoci memisahkan deklarasi variabel dengan nilai secret:
* Di `skoci.yaml`: Anda hanya menuliskan nama kuncinya (misal `DATABASE_URL`).
* Nilai sebenarnya disimpan langsung ke server Skoci dalam bentuk terenkripsi:

```bash
skoci secret set DATABASE_URL "postgresql://skoci:rahasia@localhost:5432/toko_db"
skoci secret set API_SECRET_KEY "sk-live-9876543210"
```

Secret disimpan dengan enkripsi **ChaCha20-Poly1305** di node dan **tidak pernah masuk ke konteks prompt agen AI**. Nilai secret disuntikkan langsung saat container dinyalakan.

---

## 5. Melakukan Deployment ke Preview

Deploy aplikasi Anda:
```bash
skoci deploy --env preview
```

### Urutan Alur Deploy Skoci:
1. **Validasi Kebijakan:** Skoci mengecek `skoci.policy.yaml` apakah env `preview` boleh dideploy otomatis oleh agen (`deploy: auto`).
2. **Build Kontainer:** Kode dikompilasi ke image container dengan batas CPU dan RAM.
3. **Penyalaan Kontainer Baru:** Kontainer versi baru dijalankan pada port kandidat.
4. **Health Check:** Skoci melakukan HTTP GET ke endpoint `/healthz` sampai kontainer merespons 200 OK.
5. **Zero-Downtime Cutover:** Reverse proxy menahan trafik masuk selama ~20 milidetik, menukar target routing ke kontainer baru, lalu melepas trafik.
6. **Pencatatan Bukti:** Skoci menulis file `skoci.evidence.json` yang berisi commit git, timestamp, dan status smoke test.

---

## 6. Memeriksa Status & Log

Periksa kondisi kontainer secara real-time:
```bash
skoci status
```

Periksa log keluaran kontainer (secret otomatis disamarkan sebagai `[REDACTED_SECRET]`):
```bash
skoci logs --tail 50
```

---

## 7. Promosi ke Production & Rollback

### A. Promosi ke Production (Human-in-the-Loop)
Jika `skoci.policy.yaml` mengatur `production: { deploy: approve }`, agen AI tidak bisa langsung mendeploy ke production. Agen harus meminta izin:

```bash
skoci promote "Semua tes preview lolos, siap rilis ke production"
```
Perintah mengembalikan `request_id: req_1234abcd`.

Operator manusia dapat menyetujuinya lewat **Dashboard Web** di `http://IP_SERVER:8787` (1 klik tombol "Setujui Promosi") atau lewat terminal:
```bash
skoci --token sk_master_admin approve req_1234abcd
```

### B. Rollback 1-Detik
Jika terjadi galat di versi baru, lakukan rollback instan ke versi sehat sebelumnya:
```bash
skoci rollback
```
Reverse proxy Skoci seketika mengembalikan rute trafik ke kontainer versi sehat sebelumnya tanpa jeda waktu.
