# Deploy Database & Manajemen Backup di Skoci

Skoci mendukung penyediaan database aplikasi (*database accessories*) yang berjalan sebagai kontainer dengan volume data persisten dan jaringan internal yang terisolasi.

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
server: default
run:
  port: 3000
  health: /healthz
services:
  db:
    engine: postgres
    version: "16"
    memory: 512m          # Batas memori container database
    backup:
      schedule: "0 2 * * *"  # Jadwal cron (setiap jam 02:00 malam)
      keep: 7                # Menyimpan 7 snapshot backup terakhir
      to: local              # Tujuan: local | s3
```

### Praktik Terbaik Skoci:
1. **Jaringan Internal:** Database hanya dapat diakses melalui jaringan internal bridge Docker `{app}-net`. Port database **tidak dipublikasikan ke internet publik**, mencegah serangan brute-force dari luar.
2. **Volume Persisten:** Data tersimpan pada volume persisten Docker `{app}-{service}-data`. Data tidak akan hilang saat aplikasi atau database di-restart.
3. **Penyuntikan Kredensial Otomatis:** Skoci membuat kata sandi acak berkekuatan tinggi (24 karakter alfanumerik) dan otomatis menyuntikkannya sebagai environment variable `DATABASE_URL` ke kontainer aplikasi Anda:
   ```
   postgresql://skoci:KATA_SANDI_ACAK@toko-db:5432/toko
   ```

---

## 3. Membuat Backup Database Segera (`skoci db backup`)

Selain backup otomatis sesuai jadwal cron, Anda atau agen AI dapat memicu backup seketika sebelum melakukan perubahan skema atau migrasi data:

```bash
skoci db backup db
```

Output terstruktur:
```json
{
  "success": true,
  "data": {
    "backup_id": "toko_db_20261011_024500",
    "app": "toko",
    "service": "db",
    "size_bytes": 10485760,
    "location": "/var/lib/skoci/backups/toko_db_20261011_024500.sql.gz",
    "created_at": "2026-10-11T02:45:00Z"
  },
  "summary": "Backup database berhasil dibuat"
}
```

Semua proses dump berjalan dalam kontainer sementara (*ephemeral*) dan langsung terhapus setelah dump selesai, sehingga tidak menambah beban RAM idle server Skoci.

---

## 4. Memulihkan Database (`skoci db restore`)

Pemulihan database (*restore*) adalah aksi destruktif yang berisiko menimpa data yang sedang berjalan.

Oleh karena itu, sesuai aturan keamanan `skoci.policy.yaml`, **pemulihan database selalu memerlukan persetujuan manusia**:

```yaml
production:
  db_restore: approve
```

### Alur Restore:
1. Agen atau operator meminta pemulihan:
   ```bash
   skoci db restore toko_db_20261011_024500 db
   ```
2. Jika dijalankan oleh token agen, permintaan masuk ke **Kotak Persetujuan** di Dashboard.
3. Operator manusia memvalidasi dan menyetujui pemulihan lewat dashboard atau perintah admin:
   ```bash
   skoci --token sk_master_admin approve req_restore_9876
   ```
4. Skoci memulihkan arsip data ke kontainer database.
