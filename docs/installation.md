# Persyaratan Server & Panduan Instalasi Skoci

Skoci dirancang agar dapat berjalan di perangkat komputasi apa pun yang menjalankan Linux 64-bit dengan konsumsi sumber daya minimal (<20 MB RAM).

---

## 1. Persyaratan Server

Anda membutuhkan server atau mesin Linux 64-bit yang memiliki koneksi internet dan Docker. Berikut lingkungan yang didukung:

### A. VPS (Virtual Private Server)
* **Penyedia:** Hetzner, DigitalOcean, Linode/Akamai, Vultr, AWS Lightsail, OVHcloud, Hostinger, Biznet Gio, IDCloudHost.
* **Spesifikasi Minimum:** 1 vCPU, 512 MB RAM (Skoci hanya memakai ~7.8 MB RAM, sisa 500 MB penuh untuk aplikasi pengguna).

### B. Dedicated Server
* Server fisik lengkap yang Anda kendalikan sendiri (Hetzner Bare Metal, OVH, server kantor/on-premise).

### C. Virtual Machine (VM)
* Mesin virtual Linux di infrastruktur lokal: Proxmox VE, VMware ESXi, KVM, VirtualBox, atau Multipass di laptop pengembang.

### D. Raspberry Pi (SBC 64-bit)
* **Perangkat:** Raspberry Pi 4 Model B, Raspberry Pi 5, atau SBC ARM64 setara (Orange Pi 5, Rock 5).
* **PENTING (OS 64-bit Wajib):** Harus menggunakan sistem operasi 64-bit seperti **Raspberry Pi OS 64-bit (aarch64)** atau **Ubuntu Server 64-bit ARM64**. OS 32-bit (`armv7l`) tidak didukung oleh Docker modern dan Skoci.

### E. Laptop / PC Bekas
* Laptop atau PC desktop lama yang diformat dengan Ubuntu Server, Debian, atau Fedora Server untuk dijadikan server rumahan (*homelab*).

### F. Perangkat Linux Lainnya
* Semua perangkat yang menjalankan kernel Linux modern (Ubuntu 20.04+, Debian 11+, Rocky Linux, Arch Linux) dengan arsitektur `x86_64` atau `aarch64`.

---

## 2. Cara Instalasi (1 Baris Perintah)

Jalankan perintah berikut di terminal server Anda:

```bash
curl -fsSL https://zen.ai.id/skoci/install.sh | bash
```

### Apa yang Dilakukan Skrip Ini?
1. Mendeteksi OS dan arsitektur (`linux-x86_64` atau `linux-aarch64`).
2. Memeriksa keberadaan Docker Engine. Jika belum terpasang, mengunduh dan memasang Docker resmi secara otomatis.
3. Memasang binary tunggal `skoci` ke `/usr/local/bin/skoci`.
4. Membuat direktori `/var/lib/skoci` untuk database SQLite dan `/etc/skoci` untuk token.
5. Menghasilkan Master Admin Token berkekuatan tinggi (`sk_master_admin_...`).
6. Membuat unit systemd `/etc/systemd/system/skoci.service` dan mengaktifkannya agar berjalan 24/7 dan hidup otomatis saat reboot.

---

## 3. Verifikasi Instalasi

Setelah instalasi selesai, periksa status layanan:

```bash
sudo systemctl status skoci
```

Periksa kesehatan sistem dengan perintah bawaan Skoci Doctor:
```bash
skoci doctor
```

Output:
```json
{
  "healthy": true,
  "idle_ram_mb": 7.8,
  "checks": [
    { "name": "Operating System & Architecture", "passed": true, "message": "OS: linux (x86_64)" },
    { "name": "RAM Idle Budget (<20MB)", "passed": true, "message": "Penggunaan RAM proses saat ini: 7.8 MB" },
    { "name": "Docker Engine Daemon", "passed": true, "message": "Tersambung ke Docker Engine" },
    { "name": "SQLite Database Integrity", "passed": true, "message": "Database SQLite tertanam siap dan terverifikasi" }
  ]
}
```

---

## 4. Port Jaringan & Firewall

Buka port berikut pada firewall server Anda (misal UFW di Ubuntu):

```bash
# Dashboard Validasi & API
sudo ufw allow 8787/tcp

# Embedded Reverse Proxy (Trafik aplikasi web)
sudo ufw allow 8000/tcp

# Standar HTTP & HTTPS (jika dipetakan)
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
```

---

## 5. Mengelola Layanan

* **Lihat log server:** `journalctl -u skoci -f`
* **Restart server:** `sudo systemctl restart skoci`
* **Hentikan server:** `sudo systemctl stop skoci`
* **Lokasi file database:** `/var/lib/skoci/skoci.db`
* **Lokasi token admin:** `/etc/skoci/master_token`
