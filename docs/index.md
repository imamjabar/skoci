# Skoci — Documentation

> Ultra-lightweight (<20 MB RAM), secure-by-design self-hosted deployment platform built for AI coding agents as primary operators with human validation dashboards.

---

## 4 Core Differentiators

1. **Ultra-Lightweight (<20 MB Idle RAM Target):**
   A single pure Rust binary unifies the control plane, embedded reverse proxy (ports 80 and 443), SQLite database, and dashboard. It saves 95% RAM compared to Coolify (~800 MB) and Dokploy (~600 MB), leaving low-cost $4/month VPS or Raspberry Pi resources free for your applications.

2. **AI Agents as Primary Operators:**
   The CLI and MCP (Model Context Protocol) share a single API source of truth (`skoci-api`) with 100% parity. Every output supports structured JSON (`--json`), errors include actionable remediation `{code, message, fix}`, and built-in skills are provided for various harnesses (OMP, Claude Code, Pi, Codex, OpenClaw).

3. **Dashboard for Validation, Not Configuration:**
   All application configurations live in your Git repository (`skoci.yaml`). The web dashboard acts as human Mission Control: inspect container health, verify deployment evidence (`skoci.evidence.json`), approve production promotions, and trigger 1-click rollbacks.

4. **Secure by Design:**
   - **Zero Shell Concatenation:** Docker commands use structured argument vectors (`tokio::process::Command`), immune to prompt injection.
   - **Zero SSH for Agents:** Agents communicate exclusively over HTTPS REST API using scoped per-app tokens (`sk_agent_<app>_*`).
   - **Secret Encryption:** Secrets are encrypted at rest on the node (ChaCha20-Poly1305), masked in logs, and never returned to LLM context.
   - **Server-Side Build:** Your local machine does not need Docker; container compilation runs entirely on the remote server.

---

## Platform Comparison

| Feature | Coolify | Dokploy | Kamal | **Skoci** |
| :--- | :--- | :--- | :--- | :--- |
| **Binary Architecture** | PHP/Laravel monolith (~800 MB RAM) | Node/Next.js monorepo (~600 MB RAM) | Ruby Gem CLI (stateless) | **Single pure Rust binary (<20 MB RAM)** |
| **Reverse Proxy** | Separate Traefik container | Separate Traefik container | `kamal-proxy` Go container | **Embedded Hyper/Tokio on ports 80 & 443** |
| **Internal Database** | PostgreSQL + Redis containers | PostgreSQL container | None | **Embedded SQLite (zero extra daemons)** |
| **Server Access** | Root SSH / web login | Root SSH / web login | **Requires Root SSH Keys** | **Zero SSH: Scoped HTTPS Token per App** |
| **AI Agent Operator** | Read-only MCP addon | Separate CLI (~8% API) | None | **Native CLI & MCP (100% Parity)** |
| **Error Format** | Raw log dumps | Exception stack traces | Raw log output | **Structured `{code, message, fix}`** |
| **Dashboard Role** | 100+ input forms configuration | Input form configuration | None | **Validation, Evidence, & Approvals** |
| **Raspberry Pi Support** | Heavy / frequent OOM on Pi 4 2GB | Heavy on 2GB | Lightweight (requires SSH) | **Ultra-lightweight (<20 MB, native ARM64)** |

---

## Quickstart (3 Minutes)

### 1. Install Skoci on VPS / 64-bit Raspberry Pi
Run on your server:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```
Ensure ports **80** and **443** are open in your VPS provider firewall.

### 2. Create Application Token on Server
On the server, create a scoped token for your application:
```bash
skoci token create --app toko --name laptop
```

### 3. Connect Laptop to Server
On your local developer workstation:
```bash
skoci connect --server https://skoci.<base-domain> --token sk_agent_toko_xxxx
```

### 4. Deploy from Project Repository
Navigate to your application repository directory:
```bash
cd ~/projects/toko
skoci init
skoci deploy --env preview
```

Your application is immediately live at `https://toko-preview.<base-domain>`.

---

## Documentation Index

* [Server Requirements & Installation](installation.md)
* [Deploy Your First App](deploy-first-app.md)
* [Deploy Databases & Backups](deploy-first-database.md)
* [AI Agent Integration (MCP & Skills)](agent-native.md)
* [Security Architecture & Scoped Tokens](security.md)
