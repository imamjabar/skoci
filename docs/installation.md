# Persyaratan Server & Panduan Instalasi Skoci

Skoci dirancang agar dapat berjalan di perangkat komputasi apa pun yang menjalankan Linux 64-bit dengan konsumsi sumber daya minimal (<20 MB RAM).

---

## 1. Persyaratan Server

Anda membutuhkan server Linux 64-bit yang memiliki koneksi internet dan akses Docker.

### A. VPS (Virtual Private Server)
* **Penyedia yang Teruji:** Sumopod, Hetzner, DigitalOcean, Linode/Akamai, Vultr, AWS Lightsail, OVHcloud, Hostinger, Biznet Gio, IDCloudHost.
* **Spesifikasi Minimum:** 1 vCPU, 512 MB RAM (Target konsumsi RAM Skoci <20 MB, menyisakan sebagian besar memori untuk aplikasi Anda).

### B. Dedicated Server / Homelab
* Server fisik atau PC/laptop bekas dengan instalasi Ubuntu Server, Debian, atau Fedora 64-bit.

### C. Raspberry Pi (SBC 64-bit)
* **Perangkat:** Raspberry Pi 4 Model B, Raspberry Pi 5, atau SBC ARM64 setara (Orange Pi 5, Rock 5).
* **PENTING (OS 64-bit Wajib):** Harus menggunakan sistem operasi 64-bit seperti **Raspberry Pi OS 64-bit (aarch64)** atau **Ubuntu Server 64-bit ARM64**. OS 32-bit (`armv7l`) tidak didukung.

---

## 2. Cara Instalasi Server (1 Baris Perintah)

Jalankan perintah berikut di terminal server Linux Anda:

```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### Opsi Tambahan Installer (Flags)
Anda dapat menyertakan opsi konfigurasi saat menjalankan installer:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- \
  --domain sumopod.id \
  --email admin@sumopod.id
```

| Opsi | Default | Keterangan |
| :--- | :--- | :--- |
| `--domain <domain>` | `<ip-dashed>.sslip.io` | Base domain untuk routing aplikasi (`*.domain`) dan dashboard (`skoci.domain`). |
| `--email <email>` | *(kosong)* | Email kontak untuk pendaftaran sertifikat ACME Let's Encrypt. |
| `--no-tls` | *(TLS ACME aktif)* | Menonaktifkan HTTPS otomatis (hanya melayani HTTP plain). |
| `--version <tag>` | `latest` | Tag rilis spesifik yang ingin dipasang (misal `v0.1.0`). |
| `--client-only` | *(false di Linux)* | Hanya memasang binary client tanpa mengaktifkan daemon server (default di macOS). |

---

## 3. Apa yang Dikonfigurasi di Server?

1. **Docker Engine:** Jika belum terpasang, installer memasang Docker resmi via `get.docker.com`.
2. **Pengguna Sistem `skoci`:** Layanan berjalan di bawah user non-root `skoci` yang tergabung dalam grup `docker`.
3. **Direktori Data:**
   - `/etc/skoci` (izin `0750`, milik `root:skoci`) untuk konfigurasi lingkungan.
   - `/var/lib/skoci` (izin `0750`, milik `skoci:skoci`) untuk database SQLite `skoci.db`, kunci enkripsi node, dan backup.
4. **Master Admin Token:** Disimpan dengan aman di `/etc/skoci/master_token` (izin `0640`). Token ini tidak dicetak sembarangan ke terminal.
5. **Reverse Proxy & TLS:** Mengikat port 80 dan 443 secara langsung untuk menerima trafik web publik.
6. **Layanan Latar Belakang Systemd:** Mengaktifkan unit `/etc/systemd/system/skoci.service` dengan pengerasan keamanan modern (`ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `CAP_NET_BIND_SERVICE`).

---

## 4. Instalasi Client di Laptop (macOS & Linux)

Di komputer pengembang lokal Anda, pasang binary `skoci` untuk berinteraksi dengan server:

```bash
# macOS atau Linux (Client Only)
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

Binary akan terpasang di `/usr/local/bin/skoci` (atau `~/.local/bin/skoci`).

---

## 5. Port Jaringan & Firewall

Skoci menggunakan port standar web internasional:

* **Port 80 (HTTP):** Menerima trafik HTTP dan tantangan ACME Let's Encrypt.
* **Port 443 (HTTPS):** Menerima trafik HTTPS terenkripsi.
* **Port 8787 (Control API):** **Hanya mengikat `127.0.0.1`** (tidak boleh dibuka ke publik). Akses dashboard dari luar dilakukan via domain proxy (`https://skoci.<base-domain>`) atau SSH tunnel.

Konfigurasi firewall UFW di server:
```bash
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp

# Jika sebelumnya ada aturan port 8787 atau 8000, hapus aturan tersebut:
sudo ufw delete allow 8787/tcp || true
sudo ufw delete allow 8000/tcp || true
```

> **Perhatian:** Buka port **80** dan **443** di firewall cloud penyedia VPS Anda (misal dashboard firewall Sumopod, Hetzner Cloud Firewall, atau Security Group AWS).

---

## 6. Verifikasi Kesehatan Sistem (`skoci doctor`)

Periksa kondisi seluruh subsistem Skoci:

```bash
skoci doctor
```

Output:
```json
{
  "healthy": true,
  "version": "0.1.0",
  "idle_ram_mb": 11.4,
  "checks": [
    { "name": "docker_daemon", "passed": true, "message": "Docker Engine aktif" },
    { "name": "proxy_http", "passed": true, "message": "Reverse proxy mengikat port 80" },
    { "name": "proxy_tls", "passed": true, "message": "TLS ACME aktif" },
    { "name": "data_dir", "passed": true, "message": "Direktori /var/lib/skoci dapat ditulis" },
    { "name": "sqlite_integrity", "passed": true, "message": "PRAGMA quick_check: ok" }
  ]
}
```

---

## 7. Mengelola Layanan di Server

* **Lihat log layanan:** `sudo journalctl -u skoci -f`
* **Restart server:** `sudo systemctl restart skoci`
* **Hentikan server:** `sudo systemctl stop skoci`
* **Baca Master Token:** `sudo cat /etc/skoci/master_token`
