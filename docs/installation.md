# Server Requirements & Installation Guide

Skoci is engineered to run on any 64-bit Linux system with minimal resource consumption (<20 MB RAM).

---

## 1. Server Requirements

You need a 64-bit Linux server with internet connectivity and Docker support.

### A. VPS (Virtual Private Server)
* **Tested Providers:** Sumopod, Hetzner, DigitalOcean, Linode/Akamai, Vultr, AWS Lightsail, OVHcloud, Hostinger, Biznet Gio, IDCloudHost.
* **Minimum Specifications:** 1 vCPU, 512 MB RAM (Skoci targets <20 MB idle RAM, leaving virtually all host memory for your applications).

### B. Dedicated Server / Homelab
* Physical server or refurbished PC/laptop running Ubuntu Server, Debian, or Fedora 64-bit.

### C. Raspberry Pi (64-bit SBC)
* **Hardware:** Raspberry Pi 4 Model B, Raspberry Pi 5, or equivalent ARM64 SBC (Orange Pi 5, Rock 5).
* **IMPORTANT (64-bit OS Required):** Must run a 64-bit operating system such as **Raspberry Pi OS 64-bit (aarch64)** or **Ubuntu Server 64-bit ARM64**. 32-bit operating systems (`armv7l`) are not supported.

---

## 2. Server Installation (1-Line Command)

Run the following command in your Linux server terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### Additional Installer Flags
You can pass custom configuration options to the installer:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- \
  --domain sumopod.id \
  --email admin@sumopod.id
```

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--domain <domain>` | `<dashed-ip>.sslip.io` | Base domain for routing apps (`*.domain`) and the dashboard (`skoci.domain`). |
| `--email <email>` | *(empty)* | Contact email for Let's Encrypt ACME certificate registration. |
| `--no-tls` | *(ACME TLS enabled)* | Disable automatic HTTPS (serve plain HTTP only). |
| `--version <tag>` | `latest` | Specific release tag to install (e.g. `v0.1.0`). |
| `--client-only` | *(false on Linux)* | Install client binary only without server daemon (default on macOS). |

---

## 3. Server Configuration Details

1. **Docker Engine:** Installed automatically via official `get.docker.com` script if not already present.
2. **System User `skoci`:** Runs under a non-root system user `skoci` added to the `docker` group.
3. **Data Directories:**
   - `/etc/skoci` (permissions `0750`, owned by `root:skoci`) for environment configuration.
   - `/var/lib/skoci` (permissions `0750`, owned by `skoci:skoci`) for SQLite database `skoci.db`, node encryption key, and backups.
4. **Master Admin Token:** Stored securely at `/etc/skoci/master_token` (permissions `0640`). Never printed casually to standard output.
5. **Reverse Proxy & TLS:** Directly binds ports 80 and 443 to handle public web traffic.
6. **Systemd Background Service:** Enables `/etc/systemd/system/skoci.service` with modern sandboxing hardening (`ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `CAP_NET_BIND_SERVICE`).

---

## 4. Client Installation on Workstation (macOS & Linux)

On your local developer workstation, install the `skoci` binary to interact with the remote server:

```bash
# macOS or Linux (Client Only)
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

The binary is installed into `/usr/local/bin/skoci` (or `~/.local/bin/skoci`).

---

## 5. Network Ports & Firewall

Skoci uses standard web ports:

* **Port 80 (HTTP):** Handles plain HTTP traffic and ACME challenge validation.
* **Port 443 (HTTPS):** Handles encrypted HTTPS traffic.
* **Port 8787 (Control API):** **Binds strictly to `127.0.0.1`** (never expose publicly). External dashboard/API access routes through the reverse proxy (`https://skoci.<base-domain>`) or SSH tunnel.

UFW firewall configuration on the server:
```bash
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp

# If previous rules for port 8787 or 8000 exist, remove them:
sudo ufw delete allow 8787/tcp || true
sudo ufw delete allow 8000/tcp || true
```

> **Warning:** Make sure ports **80** and **443** are open in your cloud provider firewall (e.g. Sumopod dashboard, Hetzner Cloud Firewall, or AWS Security Group).

---

## 6. System Health Verification (`skoci doctor`)

Check the health of all Skoci subsystems:

```bash
skoci doctor
```

Example output:
```json
{
  "healthy": true,
  "version": "0.1.0",
  "idle_ram_mb": 11.4,
  "checks": [
    { "name": "docker_daemon", "passed": true, "message": "Docker Engine is active" },
    { "name": "proxy_http", "passed": true, "message": "Reverse proxy bound to port 80" },
    { "name": "proxy_tls", "passed": true, "message": "ACME TLS is active" },
    { "name": "data_dir", "passed": true, "message": "Directory /var/lib/skoci is writable" },
    { "name": "sqlite_integrity", "passed": true, "message": "PRAGMA quick_check: ok" }
  ]
}
```

---

## 7. Managing the Server Service

* **View service logs:** `sudo journalctl -u skoci -f`
* **Restart service:** `sudo systemctl restart skoci`
* **Stop service:** `sudo systemctl stop skoci`
* **Read Master Token:** `sudo cat /etc/skoci/master_token`
