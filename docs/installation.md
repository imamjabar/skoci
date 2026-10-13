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

### Option A: Automated Setup from Your Laptop (Recommended, Skoci >= 0.4.0)
If you have the client installed on your laptop (Skoci >= 0.4.0), configure the entire remote VPS over SSH without typing commands manually on the server:
```bash
skoci server setup --host <server-public-ip> --user root
```
This connects over SSH, installs Docker and the Skoci systemd service, configures UFW firewall rules by default (ports SSH, 80, 443, and 8443), provisions the control plane TLS key, retrieves the SPKI pin, and automatically saves a server profile in `~/.config/skoci/servers.json` (`0600`). When `--domain` is omitted, `skoci server setup` automatically enables magic DNS (`<dashed-ip>.sslip.io`) and prints a warning so your first preview deployment immediately gets a working HTTPS URL (`https://<app>-preview.<dashed-ip>.sslip.io`). Pass `--domain <your-domain>` to configure a domain you own from the start, or switch later with `skoci domain set <your-domain> --primary`. After this step, **no SSH connection is ever required again**.

#### Server Setup Flags
You can pass flags during setup to handle password-only VPS hosts, harden SSH access, customize firewall behavior, and configure approval policies:
```bash
skoci server setup --host <server-public-ip> --user root \
  --ssh-copy-id \
  --harden-ssh
```

| Flag | Description |
| :--- | :--- |
| `--host <host>` | Target VPS address (IP literal or hostname). |
| `--user <user>` | SSH account used for bootstrap (default `root`). |
| `--port <n>` | SSH port of the target host (default `22`). Also used for the UFW rule configured during setup. |
| `--ssh-key <path>` | SSH private key to authenticate with, instead of your agent's default identity. |
| `--ssh-copy-id` | Install your SSH public key on a password-only VPS (generates `~/.ssh/id_ed25519` if you have no key), instead of failing. |
| `--ssh-password-stdin` | Read the SSH and remote `sudo -S` password from the first line of standard input for non-interactive bootstrap (never passed in `argv`, replacing `expect` wrappers). |
| `--app <app>` | Application name that the generated agent token is scoped to (defaults to `app` in `.skoci/skoci.yaml`). |
| `--version <tag>` | Specific Skoci release tag to install on the server (default `latest`, e.g. `v0.6.1`). |
| `--domain <domain>` | Base domain you own, used for app hostnames (`<app>.<domain>`, `<app>-preview.<domain>`) and dashboard (`skoci.<domain>`). |
| `--email <email>` | Contact email address for Let's Encrypt ACME certificate registration. |
| `--magic-dns` | Enable `<dashed-ip>.sslip.io` preview hostnames (enabled automatically by `server setup` when `--domain` is omitted; unsuitable for production). |
| `--harden-ssh` | Writes `/etc/ssh/sshd_config.d/00-skoci-harden.conf` disabling password and keyboard-interactive auth and setting `PermitRootLogin prohibit-password`. It sorts ahead of distro drop-ins such as `50-cloud-init.conf` because `sshd` keeps the first value it reads; validated with `sshd -t`, re-checked with `sshd -T`, reverted on failure, and skipped when the account has no `authorized_keys`. |
| `--no-ufw` | Skip UFW firewall configuration. By default, setup allows the SSH port, 80, 443, and 8443 and enables `ufw` with default deny incoming (skipped with instructions if `ufw` is not installed). |
| `--strict-approvals` | Write `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` to `/etc/skoci/skoci.env` so every production promotion requires human approval, including an application's first one. |
| `--yes` / `-y` | Do not prompt for confirmation; required when running setup from a script or an agent. |
| `--allow-magic-dns-production` | Operator override: write `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true` to `/etc/skoci/skoci.env` to allow `sslip.io` hostnames for production deploys without editing files over SSH. Prints a warning during setup because magic-DNS hostnames are refused by default in production. |

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
| `--magic-dns` | *(disabled)* | Opt-in to generate `<dashed-ip>.sslip.io` subdomains for preview convenience when running `install.sh` directly (`skoci server setup` passes this automatically when `--domain` is omitted). Unsuitable for production. |
| `--domain <domain>` | *(empty)* | Base domain for routing apps (`*.domain`) and the dashboard (`skoci.domain`). |
| `--email <email>` | *(empty)* | Contact email for Let's Encrypt ACME certificate registration. |
| `--no-tls` | *(ACME TLS enabled)* | Disable automatic HTTPS (serve plain HTTP only). |
| `--no-ufw` | *(UFW enabled)* | Skip UFW firewall configuration (UFW is configured by default when installed). |
| `--harden-ssh` | *(disabled)* | Harden `sshd` via `/etc/ssh/sshd_config.d/00-skoci-harden.conf`. |
| `--ssh-port <n>` | *(auto-detected / 22)* | SSH port to allow when configuring UFW. |
| `--strict-approvals` | *(disabled)* | Require human approval for every production promotion, including an app's first one (`SKOCI_REQUIRE_APPROVAL_ALWAYS=true`). |
| `--version <tag>` | `latest` | Specific release tag to install (e.g. `v0.6.1`). |
| `--allow-magic-dns-production` | *(disabled)* | Operator override: allow magic-DNS hostnames in production by writing `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true` to `/etc/skoci/skoci.env`. |
| `--client-only` | *(false on Linux)* | Install client binary only without server daemon (default on macOS). |

> **Why is Magic DNS (`sslip.io`) rejected in production by default?**  
> Third-party wildcard zones are operated by external third parties. Whoever controls that zone can request certificates for your hostnames, and all global users share a single Let's Encrypt rate-limit bucket. Skoci rejects production deployments that use magic DNS hostnames unless the operator explicitly enables `--allow-magic-dns-production`.
---

## 3. Server Configuration Details

1. **Docker Engine:** Installed automatically via official `get.docker.com` script if not already present.
2. **System User `skoci`:** Runs under a non-root system user `skoci` added to the `docker` group. The invoking SSH user is not added to the `skoci` group.
3. **Data Directories & TLS Keys:**
   - `/etc/skoci` (permissions `0700`, owned by `root:root`) for server configuration. Server daemon settings and `SKOCI_MASTER_TOKEN` are stored exclusively in `/etc/skoci/skoci.env` (permissions `0600`, owned by `root:root`, read by systemd). Boolean values in `/etc/skoci/skoci.env` (such as `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION`, `SKOCI_REQUIRE_APPROVAL_ALWAYS`, and `SKOCI_ACME_STAGING`) accept `true/false`, `1/0`, `yes/no`, and `on/off` (case-insensitive). Retention policy knobs configured here:
     - `SKOCI_RETENTION_DEPLOYMENTS_PER_APP`: historical deployments to keep per app (default `30`).
     - `SKOCI_RETENTION_DEPLOYMENTS_MAX_AGE_DAYS`: max age in days for deployment history (default `90`).
     - `SKOCI_RETENTION_APPROVALS_MAX_AGE_DAYS`: max age in days for decided approvals (default `30`).
     - `SKOCI_RETENTION_AUDIT_LOGS_MAX_AGE_DAYS`: max age in days for audit logs (default `90`).
     Rows referenced by an active live route or pending approval are never pruned. Additionally, container logs are bounded (`--log-opt max-size=10m --log-opt max-file=3`) and the BuildKit build cache is safely pruned hourly (keeping 5 GB).
   - `/var/lib/skoci` (permissions `0700`, owned by `skoci:skoci`) for SQLite database `skoci.db`, node encryption key `/var/lib/skoci/node.key` (permissions `0600`), and backups. The node key encrypts all stored secrets and generated accessory-database passwords. The daemon refuses to start if `node.key` is missing or unreadable while secrets exist; operators must back up `node.key` (mode `0600`). If stored secrets are lost and the operator accepts wiping them, set `SKOCI_ALLOW_SECRET_KEY_RESET=true` once to recreate the key and delete unreadable rows.
   - `/var/lib/skoci/tls/control.key` (permissions `0600`) and `/var/lib/skoci/tls/control.pin` (permissions `0644`) for the stable SPKI-pinned TLS control identity.
4. **Master Admin Token:** Stored exclusively as `SKOCI_MASTER_TOKEN` inside `/etc/skoci/skoci.env` (permissions `0600 root:root`; any separate legacy token file under `/etc/skoci/` is deleted on install or upgrade). Because `/etc/skoci/skoci.env` is root-only, server-side admin CLI commands (`sudo skoci approve`, `sudo skoci token create`, `sudo skoci server fingerprint`) require `sudo`.
5. **Client Configuration Profiles:** `skoci connect` writes server profiles to `~/.config/skoci/servers.json` (permissions `0600`) by default, and writes project-local `.skoci/env` (permissions `0600`, auto-added to `.gitignore`) only when `--project-env` is passed. Legacy `.env.skoci` and root `skoci.yaml` are migrated automatically to `.skoci/env` and `.skoci/skoci.yaml`.
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
  "version": "0.6.1",
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
* **Read Master Token:** `sudo grep '^SKOCI_MASTER_TOKEN=' /etc/skoci/skoci.env | cut -d= -f2-`
* **Run admin CLI commands on server:** prefix with `sudo` (e.g. `sudo skoci approve <id>`, `sudo skoci token create --app <app>`)
