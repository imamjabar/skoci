# Deploy Database & Manajemen Backup di Skoci

Skoci mendukung penyediaan layanan database (*database accessories*) yang berjalan sebagai kontainer dengan volume data persisten dan jaringan internal yang terisolasi.

---

## 1. Engine Database yang Didukung

* **PostgreSQL:** Versi `15`, `16`, `17` (Image resmi: `postgres:<version>`)
* **MySQL:** Versi `8.0`, `8.4` (Image resmi: `mysql:<version>`)
* **Valkey / Redis:** Versi `7.2`, `8.0` (Image resmi: `valkey/valkey:<version>`)

---

## 2. Mendeklarasikan Database di `skoci.yaml`

Database dideklarasikan di bagian `services` pada file `skoci.yaml` di repositori aplikasi Anda:

```yaml
version: 1
app: toko
run:
  port: 3000
  health: /healthz
services:
  db:
    engine: postgres
    version: "16"
    memory: 512m          # Batas memori kontainer database
    backup:
      schedule: "0 2 * * *"  # Jadwal cron UTC (setiap jam 02:00 malam)
      keep: 7                # Menyimpan 7 snapshot backup terbaru
```

### Karakteristik Jaringan & Keamanan Layanan:
1. **Jaringan Internal Terisolasi:** Kontainer database tergabung dalam jaringan bridge Docker `skoci-<app>` dan tidak memublikasikan port ke internet publik host.
2. **Volume Persisten:** Data tersimpan pada volume Docker bernama `skoci-<app>-<service>-data` (misal `skoci-toko-db-data`). Data tetap aman saat kontainer aplikasi di-deploy ulang.
3. **Penyuntikan Kredensial Otomatis:**
   - Skoci otomatis membuat kata sandi acak alfanumerik 32 karakter dan menyimpannya sebagai secret terenkripsi `SKOCI_<SERVICE_UPPER>_PASSWORD`.
   - Variabel koneksi otomatis disuntikkan ke kontainer aplikasi:
     - Layanan bernama `db` → disuntikkan sebagai `DATABASE_URL`.
     - Layanan dengan nama lain (misal `cache`) → disuntikkan sebagai `<NAMA_UPPER>_URL` (misal `CACHE_URL`).
   - Format URL koneksi:
     - **Postgres:** `postgresql://skoci:<password>@db:5432/<nama_app>`
     - **MySQL:** `mysql://skoci:<password>@<service>:3306/<nama_app>`
     - **Valkey:** `redis://:<password>@<service>:6379`

---

## 3. Membuat Backup Database Segera (`skoci db backup`)

Selain backup otomatis sesuai jadwal cron, Anda atau agen AI dapat memicu backup seketika sebelum melakukan migrasi data:

```bash
skoci db backup db
```

Output terstruktur:
```json
{
  "success": true,
  "data": {
    "backup_id": "db-20261011024500",
    "app": "toko",
    "service": "db",
    "size_bytes": 10485760,
    "location": "/var/lib/skoci/backups/toko/db/db-20261011024500.dump",
    "created_at": "2026-10-11T02:45:00Z"
  },
  "summary": "Backup database 'db' berhasil dibuat"
}
```

Format berkas backup:
* PostgreSQL: `.dump` (menggunakan `pg_dump -Fc`)
* MySQL: `.sql` (menggunakan `mysqldump`)
* Valkey: `.rdb` (menggunakan snapshot `SAVE`)

Daftar backup dapat dilihat kapan saja dengan:
```bash
skoci db list
```

---

## 4. Memulihkan Database (`skoci db restore`)

Pemulihan database (*restore*) adalah aksi destruktif yang berpotensi menimpa data yang sedang berjalan.

Secara default pada server Skoci, **pemulihan database oleh agen selalu memerlukan persetujuan operator manusia**:

### Alur Restore:
1. Agen AI atau pengembang meminta pemulihan:
   ```bash
   skoci db restore db-20261011024500 db
   ```
2. Skoci mencatat permintaan pemulihan dan mengembalikan status `pending_approval` dengan `request_id`.
3. Operator manusia memvalidasi dan menyetujui pemulihan lewat **Web Dashboard** di `https://skoci.<base-domain>` atau lewat terminal server:
   ```bash
   skoci approve req_restore_9876
   ```
4. Skoci memulihkan arsip data ke kontainer database secara atomik.
