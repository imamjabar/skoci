# Model Keamanan & Arsitektur Pertahanan Skoci

Ancaman utama pada platform deployment agent-native berbeda dengan PaaS konvensional: **input perintah datang dari agen AI yang dapat terkena serangan Prompt Injection.**

Oleh karena itu, setiap lapisan Skoci menganggap input agen sebagai data tidak tepercaya (*untrusted input*) dan menerapkan model pertahanan berlapis (*defense-in-depth*).

---

## 7 Pilar Pertahanan Skoci

```
[Agen AI / Prompt Eksternal]
          │
          ▼
┌───────────────────────────────────────────────┐
│ 1. Zero Shell String (Argv Only)              │ ◄── tokio::process::Command tanpa sh -c
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 2. Scoped Token & SHA-256 Hash                │ ◄── DB hash lookup; isolasi antar app
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 3. Zero-SSH Key Policy                        │ ◄── Tidak ada root SSH key yang disimpan
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 4. Enkripsi Secret Node (ChaCha20-Poly1305)   │ ◄── Secret disamarkan & disuntik via file 0600
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 5. Isolasi Jaringan Kontainer & Bind Lokal    │ ◄── API bind 127.0.0.1; network per app
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 6. Gerbang Persetujuan Manusia (Human Gate)   │ ◄── Kebijakan bawaan: aksi prod butuh approval
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 7. Audit Log Append-Only                      │ ◄── Pencatatan aktor; token tidak disimpan
└───────────────────────────────────────────────┘
```

---

### Pilar 1: Kebal Injeksi Shell (Zero Shell Concatenation)
* **Kelemahan Alat Tradisional:** Banyak PaaS membangun perintah terminal dengan penggabungan string:
  `sh -c "docker run -e KEY=$input ..."`
  Jika input mengandung karakter seperti `;`, `&`, `|`, atau `$()`, penyerang mendapatkan akses shell server.
* **Solusi Skoci:** Skoci **TIDAK PERNAH** memanggil shell `sh` dengan string yang diinterpolasi dari input pengguna/agen. Skoci memanggil proses biner Docker langsung dengan irisan argumen terstruktur (`tokio::process::Command::new("docker").args(...)`). Karakter khusus diperlakukan murni sebagai teks harfiah. Perintah shell yang diizinkan hanyalah string literal tetap yang hanya merujuk pada environment variable kontainer itu sendiri.

---

### Pilar 2: Scoped Token Terisolasi & Verifikasi Hash SHA-256
* Token agen berformat `sk_agent_<app>_<48 hex>` disimpan dalam database SQLite sebagai **hash SHA-256**.
* Ruang lingkup (*scope*) aplikasi ditentukan oleh baris database hasil pencarian hash, bukan dengan mem-parsing string token.
* Token aplikasi `toko` ditolak jika mencoba mengakses aplikasi `blog` (`scope_violation`).
* **Pencabutan Instan:** Admin dapat mencabut token kapan saja:
  ```bash
  skoci token revoke --app toko --name laptop
  ```
* **Proteksi Brute-Force:** Gagal autentikasi dibatasi maksimal 10 kali per menit per IP peer sebelum diblokir dengan status `rate_limited`.

---

### Pilar 3: Kebijakan Zero-SSH (Tanpa Kunci SSH Root)
* Alat seperti Kamal mewajibkan developer dan runner CI menyimpan kunci SSH root server. Jika mesin developer atau sandbox agen AI diretas, penyerang langsung menguasai seluruh server fisik.
* **Skoci meniadakan SSH untuk operasional:** Agen hanya berkomunikasi melalui protokol HTTPS REST API dan MCP yang diotentikasi token aplikasi.

---

### Pilar 4: Enkripsi Secret di Node & Penyuntikan Aman
* Secret dienkripsi saat tersimpan di database menggunakan **ChaCha20-Poly1305** dengan kunci node `/var/lib/skoci/node.key` (32 byte acak dibuat saat start pertama, izin berkas `0600`, hanya bisa dibaca user `skoci`).
* Agen AI dapat menulis nilai (`skoci secret set KEY VAL`), tetapi **tidak pernah bisa membaca nilainya kembali** (`skoci secret list` hanya mengembalikan nama kunci). Nilai secret yang mengandung karakter newline (`\n`) ditolak.
* **Penyuntikan Bebas Bocor:** Saat kontainer dijalankan, secret ditulis ke file environment sementara dengan izin `0600` dan dioperkan via `docker run --env-file`. File tersebut langsung dihapus dari disk seketika setelah kontainer berjalan.
* **Sanitasi Log:** Nilai rahasia dengan panjang >= 4 karakter otomatis disaring dan disamarkan dari seluruh aliran log kontainer menjadi `[REDACTED]`.

---

### Pilar 5: Isolasi Bind Lokal & Jaringan Kontainer
* **Bind 127.0.0.1:8787:** Control API Skoci hanya mengikat interface lokal `127.0.0.1:8787` dan tidak terbuka langsung ke publik. Akses luar wajib melalui reverse proxy ber-TLS atau SSH tunnel.
* **Session Cookie Dashboard:** Dashboard web menggunakan cookie `skoci_session` (HttpOnly, SameSite=Strict, Secure saat HTTPS) berumur 12 jam, dan mewajibkan header CSRF `X-Skoci-Dashboard: 1` untuk semua permintaan mutasi.
* **Perbandingan Waktu Konstan:** Master token dibandingkan menggunakan algoritma waktu konstan (*constant-time comparison*) untuk mencegah serangan *timing attack*.
* **Isolasi Jaringan Kontainer:** Setiap aplikasi dan databasenya berada di dalam Docker bridge network tersendiri (`skoci-<app>`). Kontainer aplikasi `toko` tidak dapat menjangkau atau mengganggu kontainer database `blog`.

---

### Pilar 6: Pengerasan Systemd & Catatan Grup Docker
Service Skoci dijalankan dengan unit systemd berfitur pengerasan (*sandboxing*):
* `User=skoci`, `Group=skoci`
* `NoNewPrivileges=true`
* `ProtectSystem=strict`
* `ProtectHome=true`
* `PrivateTmp=true`
* `ReadWritePaths=/var/lib/skoci`
* `AmbientCapabilities=CAP_NET_BIND_SERVICE` (agar proxy dapat mengikat port 80 dan 443 tanpa hak akses root penuh).

> **Catatan Keamanan (Docker Group Caveat):**  
> Pengguna atau layanan yang tergabung dalam grup `docker` memiliki akses ke socket `/var/run/docker.sock`. Pada Linux, akses ke Docker socket secara fungsional setara dengan root (`docker group ≈ root`). Skoci mengisolasi pemanggilan proses dan memperketat validasi argumen untuk meminimalkan risiko ini.

---

### Pilar 7: Batas Upload & Audit Log Append-Only
* **Batas Upload:** Unggahan arsip dibatasi sebesar 200 MB (`--max-upload-mb`) untuk melindungi server dari serangan penipisan memori/disk (*resource exhaustion*).
* **Audit Log:** Seluruh aktivitas dicatat dalam tabel `audit_logs` SQLite dengan identitas aktor (`admin` atau `agent:<app>/<name>#<id12>`). Log audit tidak pernah menyimpan plain-text token.
