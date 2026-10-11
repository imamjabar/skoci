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

## 2. Server Installation

### Option A: Automated Setup from Your Laptop (Recommended)
If you have client installed on your laptop, configure the entire remote VPS over SSH without typing commands manually on the server:
```bash
skoci server setup --host <server-public-ip> --user root
```
This connects over SSH, installs Docker and the Skoci systemd service, provisions the control plane TLS key, retrieves the SPKI pin, and automatically saves a server profile in `~/.config/skoci/servers.json`. After this step, **no SSH connection is ever required again**.

### Option B: 1-Line Command on the Server
Alternatively, log in to your Linux server terminal and run:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### Additional Installer Flags
You can pass custom configuration options to the installer:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- \
  --email admin@example.com
```

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--magic-dns` | *(disabled)* | Opt-in to generate temporary `<dashed-ip>.sslip.io` subdomains for preview convenience. Unsuitable for production. |
| `--domain <domain>` | *(empty)* | Base domain for routing apps (`*.domain`) and the dashboard (`skoci.domain`). |
| `--email <email>` | *(empty)* | Contact email for Let's Encrypt ACME certificate registration. |
| `--no-tls` | *(ACME TLS enabled)* | Disable automatic HTTPS (serve plain HTTP only). |
| `--version <tag>` | `latest` | Specific release tag to install (e.g. `v0.4.0`). |
| `--client-only` | *(false on Linux)* | Install client binary only without server daemon (default on macOS). |

> **Why is Magic DNS (`sslip.io`) opt-in only?**  
> Third-party wildcard zones are operated by external third parties. Whoever controls that zone can request certificates for your hostnames, and all global users share a single Let's Encrypt rate-limit bucket. Skoci rejects production deployments that use magic DNS hostnames.
---

## 3. Server Configuration Details

1. **Docker Engine:** Installed automatically via official `get.docker.com` script if not already present.
2. **System User `skoci`:** Runs under a non-root system user `skoci` added to the `docker` group.
3. **Data Directories & TLS Keys:**
   - `/etc/skoci` (permissions `0750`, owned by `root:skoci`) for environment configuration and server settings.
   - `/var/lib/skoci` (permissions `0750`, owned by `skoci:skoci`) for SQLite database `skoci.db`, node encryption key, and backups.
   - `/var/lib/skoci/tls/control.key` (permissions `0600`) and `/var/lib/skoci/tls/control.pin` (permissions `0644`) for the stable SPKI-pinned TLS control identity.
4. **Master Admin Token:** Stored securely at `/etc/skoci/master_token` (permissions `0640`). Never printed casually to standard output.
5. **Client Configuration Profiles:** Client credentials and server profiles live in `~/.config/skoci/servers.json` (permissions `0600`). Project-local settings use `.env.skoci` (permissions `0600`, auto-added to `.gitignore`).
6. **Reverse Proxy & TLS:** Directly binds ports 80 and 443 for public app web traffic, and port 8443 for pinned control plane traffic.
7. **Systemd Background Service:** Enables `/etc/systemd/system/skoci.service` with modern sandboxing hardening (`ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `CAP_NET_BIND_SERVICE`, `UMask=0077`).

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

Skoci uses the following network ports:

* **Port 80 (HTTP):** Handles plain HTTP traffic, automatic HTTP-to-HTTPS redirects, and ACME challenge validation.
* **Port 443 (HTTPS):** Handles encrypted HTTPS traffic for applications.
* **Port 8443 (Control API TLS):** Pinned TLS control plane endpoint (`https://<public-ip>:8443`). CLI and dashboard management connections connect here using SPKI pinning.
* **Port 8787 (Control API Plain):** **Binds strictly to `127.0.0.1`** (never expose publicly) for local service communication.

UFW firewall configuration on the server:
```bash
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 8443/tcp

# If previous rules for port 8787 or 8000 exist, remove them:
sudo ufw delete allow 8787/tcp || true
sudo ufw delete allow 8000/tcp || true
```

> **Warning:** Make sure ports **80**, **443**, and **8443** are open in your cloud provider firewall (e.g. Sumopod dashboard, Hetzner Cloud Firewall, DigitalOcean Cloud Firewall, or AWS Security Group).

## 6. System Health Verification (`skoci doctor`)

Check the health of all Skoci subsystems:

```bash
skoci doctor
```

Example output:
```json
{
  "healthy": true,
  "version": "0.4.0",
  "idle_ram_mb": 11.4,
  "checks": [
    { "name": "docker_daemon", "passed": true, "message": "Docker Engine is active" },
    { "name": "proxy_http", "passed": true, "message": "Reverse proxy bound to port 80" },
    { "name": "proxy_tls", "passed": true, "message": "ACME TLS is active" },
    { "name": "control_tls", "passed": true, "message": "Control plane TLS active on port 8443" },
    { "name": "data_dir", "passed": true, "message": "Directory /var/lib/skoci is writable" },
    { "name": "sqlite_integrity", "passed": true, "message": "PRAGMA quick_check: ok" }
  ],
  "control_tls": {
    "spki_pin": "sha256:7b5fa1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcd",
    "mode": "acme-ip",
    "not_after": "2026-11-15T00:00:00Z",
    "last_error": null
  }
}
```

---

## 7. Managing the Server Service

* **View service logs:** `sudo journalctl -u skoci -f`
* **Restart service:** `sudo systemctl restart skoci`
* **Stop service:** `sudo systemctl stop skoci`
* **Read Master Token:** `sudo cat /etc/skoci/master_token`
