# Model Keamanan & Arsitektur Pertahanan Skoci

Ancaman utama pada platform deployment agent-native berbeda dengan PaaS konvensional: **input perintah datang dari agen AI yang dapat terkena serangan Prompt Injection.**

Oleh karena itu, setiap lapisan Skoci menganggap input agen sebagai data tidak tepercaya (*untrusted input*) dan menerapkan model pertahanan berlapis (*defense-in-depth*).

---

## 7 Lapisan Pertahanan Skoci

```
[Agen AI / Prompt Luar]
          │
          ▼
┌─────────────────────────────────┐
│ 1. Validasi Skema & No Shell    │ ◄── Enforce regex nama [a-z0-9_-], tanpa sh -c
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 2. Scoped Token Per Aplikasi    │ ◄── Token 'toko' ditolak jika menyentuh 'blog'
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 3. Zero-SSH Key Policy          │ ◄── Tidak ada root SSH key yang disimpan
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 4. Enkripsi Secret (ChaCha20)   │ ◄── Nilai secret tidak pernah masuk konteks LLM
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 5. Isolasi Jaringan Kontainer   │ ◄── Docker bridge network per proyek terisolasi
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 6. Gerbang Persetujuan Manusia  │ ◄── Aksi produksi wajib persetujuan operator
└────────────────┬────────────────┘
                 │
                 ▼
┌─────────────────────────────────┐
│ 7. Audit Log Append-Only        │ ◄── Rekam jejak tidak dapat dimanipulasi
└─────────────────────────────────┘
```

---

### Lapisan 1: Kebal Injeksi Shell (Zero Shell Concatenation)
* **Kelemahan Alat Tradisional (CVE-2026-34597):** Banyak PaaS membangun perintah terminal dengan penggabungan string:
  `sh -c "docker run -e KEY=$input ..."`
  Jika input mengandung karakter seperti `;`, `&`, `|`, atau `$()`, penyerang mendapatkan akses shell server.
* **Solusi Skoci:** Skoci **TIDAK PERNAH** memanggil shell `sh` atau `bash`. Skoci memanggil proses biner Docker langsung dengan irisan argumen terstruktur (`tokio::process::Command::new("docker").arg(...)`). Karakter spesial shell diperlakukan murni sebagai teks harfiah.

### Lapisan 2: Kebijakan Zero-SSH (Tanpa Kunci SSH Root)
* Alat seperti Kamal mewajibkan developer dan CI menyimpan kunci SSH root server. Jika mesin developer atau sandbox agen AI diretas, penyerang langsung menguasai seluruh server fisik.
* **Skoci mengharamkan SSH untuk agen.** Agen hanya berkomunikasi melalui protokol HTTPS REST API dan MCP yang diotentikasi token aplikasi.

### Lapisan 3: Token Scoped (Isolasi Antar Proyek)
Jika Anda menampung banyak proyek pada satu server VPS:
* Token `sk_agent_toko_*` hanya memiliki hak akses ke aplikasi `toko`.
* Jika token tersebut mencoba membaca log, mengubah secret, atau mendeploy aplikasi `blog`, Skoci seketika menolak permintaan:
  ```json
  {
    "code": "scope_violation",
    "message": "Token ini hanya memiliki izin untuk aplikasi 'toko', bukan 'blog'",
    "fix": "Gunakan token yang dibuat khusus untuk 'blog'"
  }
  ```

### Lapisan 4: Enkripsi Secret di Tempat & Sanitasi Log
* Secret dienkripsi saat tersimpan di database node menggunakan algoritma modern **ChaCha20-Poly1305** dengan kunci node hardware.
* Agen AI dapat menulis nilai (`skoci secret set KEY VAL`), tetapi **tidak pernah bisa membaca nilainya kembali** (`skoci secret list` hanya mengembalikan nama kunci).
* Nilai rahasia otomatis disaring dan disamarkan dari semua aliran log kontainer:
  `Error: DB conn failed password [REDACTED_SECRET]`

### Lapisan 5: Isolasi Jaringan Kontainer
* Setiap aplikasi dan databasenya berada di dalam Docker bridge network tersendiri (`{app}-net`).
* Kontainer aplikasi `toko` tidak dapat menjangkau atau merusak kontainer database `blog`.

### Lapisan 6: Gerbang Persetujuan Manusia (Human-in-the-Loop)
* Sesuai file kebijakan `skoci.policy.yaml`, aksi destruktif atau peluncuran ke production (`promote`, `db_restore`, `delete`) mutlak membutuhkan verifikasi manusia.
* Token agen yang mencoba memanggil endpoint persetujuan (`/api/approve`) akan ditolak dengan error `401 Unauthorized: Hanya manusia (admin) yang dapat menyetujui approval`.

### Lapisan 7: Audit Log Terstruktur
* Semua aktivitas (siapa yang memicu, kapan, aksi apa, pada aplikasi mana) dicatat dalam tabel `audit_logs` SQLite dengan mekanisme append-only dan ditampilkan transparan di Dashboard Validasi.
