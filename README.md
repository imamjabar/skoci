English | [Bahasa Indonesia](README.id.md)

# ⛵ Skoci

> **Let your AI coding agent ship to your own VPS — safely.**  
> Skoci is an ultra-lightweight, agent-native deploy platform built for Claude Code, OMP, and Cursor-style agents. It compiles and runs containers on your server with automated HTTPS, isolated tokens, and human approval gates — with an idle footprint of ~9 MB RSS.

```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

---

## What You Get

* **One-Command Agent Deploys:** Deploy directly from your agent with `skoci deploy`. No SSH access needed, and no Docker daemon required on your laptop — the server builds the image.
* **Live Progress Streaming:** Real-time build logs, health checks, and proxy cutover events. Static sites deploy in ~10–20 s after the first build.
* **Scoped Tokens, Not Root SSH:** Per-app tokens (`sk_agent_<app>_*`) replace root SSH keys. Tokens are verified via SHA-256 hashes in SQLite and isolated to their specific app.
* **Human Approval Gates:** Production promotions, database restores, and app deletions require human sign-off via web dashboard or master CLI.
* **Encrypted Secrets:** Environment secrets are encrypted at rest with ChaCha20-Poly1305, injected through ephemeral files, masked in logs, and never returned to the agent context.
* **Automatic HTTPS:** Embedded reverse proxy provisions Let's Encrypt TLS certificates automatically with free `*.sslip.io` domains or custom hostnames.
* **Zero-Downtime Cutover & Rollback:** Atomic proxy route switching after health check verification, with one-command rollback (`skoci rollback`) if anything fails.
* **Batteries-Included Databases:** Declarative Postgres, MySQL, and Valkey services with cron-scheduled backups, on-demand snapshots (`skoci db backup`), and approval-gated restores.
* **Self-Correcting Agents:** Structured JSON output (`--json`), actionable `fix` hints, and consistent exit codes enable agents to diagnose and fix errors autonomously.
* **Ultra-Lightweight Binary:** A single pure Rust binary consuming ~9 MB idle RSS (measured on Linux release build), leaving virtually all VPS memory for your workloads.

---

## Best For / Not For

### Best For
* Solo builders and vibe coders shipping side projects and MVPs from Claude Code, OMP, or Cursor-style coding agents.
* Small teams hosting several applications on a single affordable VPS (Sumopod, Hetzner, DigitalOcean, a Raspberry Pi at home).
* Anyone who wants AI agents to deploy and monitor software without handing them root SSH keys.

### Not For
* Multi-server clusters or Kubernetes-scale distributed orchestration.
* Teams that require cross-machine build caches or external managed container registries.
* Non-HTTP workloads (e.g. raw UDP/TCP protocols or batch HPC jobs).

---

## How It Works

```
┌──────────────┐          ┌───────────────────────────────────────────────┐
│   AI Agent   │          │               Skoci Server (VPS)              │
│ (Claude/OMP) │          │                                               │
└──────┬───────┘          └───────────────────────┬───────────────────────┘
       │                                          │
       │ 1. skoci deploy                          │
       │    (packs repo, honours .gitignore)      │
       ├─────────────────────────────────────────►│
       │    HTTPS upload with scoped token        │ 2. Server builds Docker image
       │                                          │ 3. Starts container & polls health check
       │                                          │ 4. Atomic proxy cutover (ports 80/443)
       │ 5. Deployment live + public URL          │
       │◄─────────────────────────────────────────┤
```

---

## Quickstart

### 1. Install on VPS
Run on your 64-bit Linux server (open ports **80** and **443** in your cloud firewall):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### 2. Create Scoped Token (on VPS)
Generate an agent token scoped strictly to your application:
```bash
sudo skoci token create --app my-app --name laptop
```

### 3. Connect Laptop to Server
On your workstation, install the client and connect your workspace (macOS installs the client only; on a Linux laptop add `--client-only`):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only
skoci connect --server https://skoci.<base-domain> --token <token>
```

### 4. Initialize & Deploy to Preview
Inside your project repository:
```bash
skoci init
skoci deploy --env preview
```

### 5. Promote to Production & Approve
Request a production promotion:
```bash
skoci promote "All preview tests passed, ready for production"
```
The command outputs exit code 2 and a `request_id`. Approve it via the web dashboard at `https://skoci.<base-domain>` or on the server:
```bash
sudo skoci approve <request_id>
```

---

## Give It to Your Agent

Copy and paste this prompt snippet into your AI coding assistant (Claude Code, OMP, Cursor, etc.):

```text
You can deploy and manage this application using the Skoci CLI.
Operational Rules:
1. Always run commands with `--json` for structured output (e.g. `skoci deploy --json`).
2. If a command fails, inspect `error.code` and execute the command provided in `error.fix`.
3. Exit code 2 means human approval is required (production deploy, db restore, app delete). Report the `request_id` to the user so they can approve via dashboard or `skoci approve`.
4. Never attempt or request SSH access. All operations use HTTPS via the configured scoped token.
5. Never print, log, or commit tokens (`sk_agent_*`, `sk_master_*`).
```

---

## Security

* **No Shell Injection:** Docker and system commands run as structured argument vectors (`tokio::process::Command`); user or agent input is never interpolated into a shell string.
* **Isolated Scoped Tokens & Stdin Entry:** Agent tokens (`sk_agent_<app>_*`) are hashed with SHA-256; app boundaries are enforced in SQLite. Tokens and secrets can be piped via stdin to prevent leaking into process listings (`ps`, `/proc`) or shell history.
* **Zero-SSH Policy:** No private SSH keys stored in developer environments or agent contexts.
* **Encrypted Secrets & Cleaned Envs:** Secrets are encrypted with ChaCha20-Poly1305 on the node, injected via ephemeral `0600` env files unlinked immediately, and masked as `[REDACTED]` in logs. Client `.env.skoci` is restricted to an allowlist (`SKOCI_SERVER_URL`, `SKOCI_TOKEN`), and spawned subprocesses run with sanitized environments.
* **Packaging Deny List:** `skoci deploy` excludes sensitive files (`.env*`, `*.pem`, `*.key`, `id_*`, `.npmrc`, `.aws/`, `.ssh/`) by default, and uploads use private `0600` temporary archives.
* **Local Control Plane & HTTPS SNI Binding:** API binds strictly to `127.0.0.1:8787`. On public HTTPS, requests enforce strict TLS SNI matching against `:authority`/`Host` headers (rejecting mismatches with `421 Misdirected Request`).
* **Host Ownership Protection:** Custom domains and auto hosts are strictly partitioned; cross-app host collisions or hijacking are rejected.
* **Human Approval Gates & Image ID Pinning:** Production releases, database restorations, and destructive operations enforce human sign-off pinned to immutable Docker image IDs (preventing TOCTOU tampering).
* **Systemd Hardening:** Service runs under a sandboxed unit with `UMask=0077`, `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `PrivateDevices=true`, `ProtectKernelTunables=true`, `ProtectKernelModules=true`, `ProtectControlGroups=true`, `RestrictSUIDSGID=true`, `LockPersonality=true`, `RestrictNamespaces=true`, and `LimitNOFILE=65536`.
* **Audit Log & Build Provenance:** Mutating operations are recorded in an append-only audit log. Official release binaries provide GitHub Actions build provenance attestations (`gh attestation verify`).

### Residual Risks

* **Docker Group ≈ Root:** Access to `/var/run/docker.sock` is root-equivalent on Linux; Skoci hardens process execution, but Docker socket privileges remain root-level.
* **Unrestricted Container Outbound Network:** Containers execute user/agent code inside standard Docker sandboxes with unrestricted outbound internet connectivity.
* **First-Come Custom Domains:** An agent can claim any custom domain not yet registered by another application on the instance.
* **Same-Origin Release Checksums:** Release checksums are hosted alongside binary assets; verify provenance with `gh attestation verify skoci-linux-x86_64 --repo imamjabar/skoci`.
* **Single-Server Docker Isolation:** Isolation relies on Linux kernel namespaces and cgroups (Docker defaults) without hardware microVM isolation.
* **ACME & sslip.io Rate Limits:** Automatic TLS for `*.sslip.io` subdomains shares global Let's Encrypt rate limits across all users of `sslip.io`. Use custom domains for production reliability.

---

## Upgrading from 0.1

If you are running an existing Skoci 0.1 installation:
1. **Clear Port 80/443 Conflicts:** Remove any older manual containers binding web ports:
   ```bash
   sudo docker ps --filter publish=80
   sudo docker rm -f <container_name>
   ```
2. **Re-run the Installer:** Update the binary and systemd service:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
   ```
3. **Recreate Scoped Tokens:** Legacy 0.1 agent tokens are revoked automatically on startup because they were exposed in a deprecated audit endpoint. Create fresh tokens:
   ```bash
   sudo skoci token create --app <app> --name laptop
   ```
4. **Reconnect & Update Config:** Reconnect your workstation (`skoci connect`) and re-set secrets (`skoci secret set`). Update `skoci.yaml`: `env.public` is now a map (`KEY: value`), `build.mode` and `server` are obsolete, and `skoci.policy.yaml` can be deleted.

---

## Documentation

* [Server Requirements & Installation Guide](docs/installation.md)
* [Deploy Your First Application](docs/deploy-first-app.md)
* [Deploy Databases & Manage Backups](docs/deploy-first-database.md)
* [AI Coding Agents as Operators (MCP & Skills)](docs/agent-native.md)
* [Security Model & Defense Architecture](docs/security.md)

---

## License

Distributed under the Apache-2.0 License. See [LICENSE](LICENSE) for details.
