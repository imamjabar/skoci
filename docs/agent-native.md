# Agen Coding Sebagai Operator (Agent-Native Architecture)

Alat deploy tradisional (seperti Coolify dan Dokploy) dirancang untuk diklik manusia melalui form antarmuka web, sementara integrasi agen AI hanyalah pelengkap eksternal.

**Skoci dirancang sejak hari pertama dengan prinsip: Agen AI yang men-deploy dan memantau, Manusia yang memvalidasi dan menyetujui.**

---

## 1. Integrasi MCP (Model Context Protocol)

Skoci menyediakan server MCP bawaan yang berkomunikasi melalui standar `stdio` JSON-RPC 2.0.

### A. Konfigurasi Claude Code (`.mcp.json` atau `~/.claude.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "https://skoci.<base-domain>",
        "SKOCI_TOKEN": "sk_agent_toko_xxxxxxxxxxxx"
      }
    }
  }
}
```

### B. Konfigurasi OMP (`~/.omp/agent/config.yml` atau `.omp/mcp.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "https://skoci.<base-domain>",
        "SKOCI_TOKEN": "sk_agent_toko_xxxxxxxxxxxx"
      }
    }
  }
}
```

### C. Penggunaan Tanpa Konfigurasi MCP (Auto-Detect via `.env.skoci`)
Jika Anda telah menjalankan `skoci connect`, berkas `.env.skoci` di direktori proyek akan otomatis dibaca oleh `skoci mcp` tanpa perlu mendefinisikan environment variable manual.

---

## 2. Daftar Tool MCP yang Tersedia

Semua tool dihasilkan dari satu sumber kebenaran API (`skoci-api`), menjamin paritas 100% dengan CLI:

| Nama Tool | Fungsi | Parameter |
| :--- | :--- | :--- |
| `skoci_init` | Mendeteksi framework & membuat `skoci.yaml` | `app`, `path` |
| `skoci_deploy` | Mengunggah, mengompilasi, & cutover | `app`, `env` (`preview`/`production`), `commit` |
| `skoci_status` | Memeriksa kondisi kontainer & kesehatan | `app` |
| `skoci_logs` | Mengambil log terbaru (secret disamarkan) | `app`, `tail` |
| `skoci_secret_set` | Menyimpan secret terenkripsi ChaCha20 | `app`, `key`, `value` |
| `skoci_secret_list` | Melihat daftar nama kunci secret | `app` |
| `skoci_promote` | Mengajukan izin promosi ke production | `app`, `reason` |
| `skoci_rollback` | Kembali ke versi stabil sebelumnya | `app`, `target_version` |
| `skoci_doctor` | Memeriksa RAM idle (<20MB) & daemon | *(tanpa parameter)* |
| `skoci_db_backup` | Memicu snapshot backup database | `app`, `service` |
| `skoci_db_restore` | Meminta pemulihan database (butuh approval) | `app`, `service`, `backup_id` |

---

## 3. Batas Keamanan Tool (Strict Human Exclusion)

Aksi persetujuan (`approve`) **secara sengaja DIKECUALIKAN dari definisi tool agen**:
1. Agen **TIDAK BISA** menyetujui permintaannya sendiri. Persetujuan promosi production mutlak berada di tangan operator manusia via Dashboard atau token master.
2. Jika agen mencoba memanggil endpoint persetujuan (`/api/approve`), sistem akan menolak dengan error `unauthorized`.

---

## 4. Standar Kode Keluar (Exit Codes)

Skoci mengembalikan kode keluar yang konsisten untuk automasi agen:

| Exit Code | Nama | Deskripsi | Tindakan Agen |
| :---: | :--- | :--- | :--- |
| `0` | `Success` | Perintah berhasil tuntas dieksekusi. | Lanjutkan alur kerja berikutnya. |
| `1` | `General Error` | Kegagalan build, health check timeout, atau validasi config. | Baca kolom `fix` pada error JSON dan perbaiki masalahnya. |
| `2` | `Approval Pending` | Aksi membutuhkan persetujuan manusia (`promote`, `db restore`). | Catat `request_id`, laporkan kepada manusia untuk disetujui di dashboard. |
| `3` | `Auth Error` | `unauthorized`, `scope_violation`, atau `rate_limited`. | Periksa token dan cakupan hak akses aplikasi. |

---

## 5. Penanganan Error `{code, message, fix}`

Setiap kali terjadi galat, Skoci mengembalikan format terstruktur yang dapat dipahami langsung oleh model LLM:

```json
{
  "success": false,
  "error": {
    "code": "secret_missing",
    "message": "Secret 'DATABASE_URL' belum diset",
    "fix": "skoci secret set DATABASE_URL <value>"
  },
  "summary": "secret_missing: Secret 'DATABASE_URL' belum diset"
}
```

Agen membaca kolom `fix` dan dapat langsung mengeksekusi instruksi perbaikan tanpa menebak penyebab kegagalan.

---

## 6. Aturan Inti untuk Agen Coding

1. **Gunakan `--json`:** Selalu sertakan flag `--json` untuk menghasilkan keluaran yang terstruktur.
2. **Jangan Minta SSH:** Setelah server terpasang, agen tidak boleh meminta atau menjalankan akses SSH. Semua interaksi berjalan via REST API/MCP.
3. **Jangan Cetak Token:** Jangan pernah menampilkan atau menulis token rahasia ke log percakapan.
4. **Baca Rekomendasi `fix`:** Ketika terjadi galat, ikuti instruksi yang tertera pada kolom `fix`.
