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
* **Live Progress & Duration Tracking:** Real-time build logs, per-stage durations, progress bar with ETA, and proxy cutover events.
* **Pinned Control TLS & Scoped Tokens:** The control plane connects over HTTPS on port `8443` pinned to an immutable SPKI public key (`sha256:...`). Per-app tokens (`sk_agent_<app>_*`) replace root SSH keys.
* **Human Approval Gates:** Production promotions, database restores, and app deletions require human sign-off via web dashboard (`https://<server-ip>:8443`) or master CLI.
* **Encrypted Secrets:** Environment secrets are encrypted at rest with ChaCha20-Poly1305, injected through ephemeral files, masked in logs, and never returned to the agent context.
* **Automatic HTTPS & Domain Verification:** Embedded reverse proxy provisions Let's Encrypt TLS certificates. Pre-validate DNS records before certificate issuance with `skoci domain check <host>`. Production applications require custom domains (magic DNS `*.sslip.io` is strictly opt-in via `--magic-dns` and rejected in production due to third-party zone control and shared rate limits).
* **Three Deployment Result States:** Deployments verify root route accessibility via smoke tests: `live` (fully operational), `degraded` (health check passed, but smoke test `GET /` failed; inspect with `skoci logs`), or `failed`. Rollback with one command (`skoci rollback`).
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
       │    (packs repo, honours .skoci/ignore)   │
       ├─────────────────────────────────────────►│
       │    HTTPS upload with scoped token        │ 2. Server builds Docker image
       │                                          │ 3. Starts container & polls health check
       │                                          │ 4. Atomic proxy cutover (ports 80/443)
       │ 5. Deployment live + public URL          │
       │◄─────────────────────────────────────────┤
```

---

## Quickstart

### 1. Install or Setup VPS
Configure your remote VPS directly from your laptop over SSH (Skoci >= 0.4.0):
```bash
skoci server setup --host <server-public-ip> --user root
```
Or install directly on your 64-bit Linux server terminal (ensure ports **80**, **443**, and **8443** are open in your firewall):
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```

### 2. Create Scoped Token and Read the Pin (on VPS)
If installing manually on the server, generate an agent token scoped strictly to your application and read the server's key pin:
```bash
sudo skoci token create --app my-app --name laptop
sudo skoci server fingerprint
```
Both commands work without extra flags on the server: the CLI reads `/etc/skoci/skoci.env` and targets the local control plane. `skoci server setup` does this for you and never prints the token.

### 3. Connect Laptop to Server
On your workstation, install the client and link your workspace using the server IP, token, and SPKI certificate pin:
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s -- --client-only
skoci connect --server <server-public-ip> --token <token> --pin sha256:<spki-pin>
```
Profiles are saved in `~/.config/skoci/servers.json` (0600) and `.skoci/env` (0600, added to `.gitignore`). Legacy `.env.skoci` is still read for projects created before 0.4.1.

### 4. Initialize & Deploy to Preview
Inside your project repository:
```bash
skoci init
skoci deploy --env preview
```
`skoci init` keeps your project root clean — everything it generates lives in `.skoci/`:
```
your-project/
├── .gitignore        # gains one line: .skoci/env
├── your files …      # untouched
└── .skoci/
    ├── skoci.yaml    # app config
    ├── Dockerfile    # generated recipe (skipped if you already have your own)
    ├── ignore        # upload filter; your .dockerignore is used when this is absent
    └── env           # server URL, token, pin (0600, never uploaded)
```
A `skoci.yaml` or `Dockerfile` written by an earlier version, or by you, is still read from the project root.

### 5. Promote to Production & Approve
Production needs a domain you own. Point an A record at the server, add it to `.skoci/skoci.yaml` under `domains:`, verify it, then request the promotion:
```bash
skoci domain check app.yourdomain.com
skoci promote "All preview tests passed, ready for production"
```
Run from a scoped agent token, the command stops at exit code 2 and prints a `request_id`; the promotion waits for a human. Approve it via the web dashboard at `https://<server-public-ip>:8443` or on the server:
```bash
sudo skoci approve <request_id>
```
The master token on the server is the human: it promotes directly, without an approval request.

---

## Give It to Your Agent

Copy and paste this prompt snippet into your AI coding assistant (Claude Code, OMP, Cursor, etc.):

```text
Operational Rules:
1. Always run commands with `--json` for structured output (e.g. `skoci deploy --json`).
2. If a command fails, inspect `error.code` and execute the command provided in `error.fix`.
3. Exit code 2 means human approval is required (production deploy, db restore, app delete). Report the `request_id` to the user so they can approve via dashboard at https://<server-ip>:8443 or `skoci approve <request_id>`.
4. Never attempt or request SSH access. All operations use pinned HTTPS via the configured scoped token.
5. Production deployments require genuine custom domains (magic DNS *.sslip.io is rejected). Verify DNS before deploying with `skoci domain check <host>`.
6. Treat `degraded` deployments as a failure to investigate: check logs with `skoci logs --tail 100 --json` and never promote a degraded version.
7. Never print, log, or commit tokens (`sk_agent_*`, `sk_master_*`).

---

## Security

* **No Shell Injection:** Docker and system commands run as structured argument vectors (`tokio::process::Command`); user or agent input is never interpolated into a shell string.
* **Isolated Scoped Tokens & Stdin Entry:** Agent tokens (`sk_agent_<app>_*`) are hashed with SHA-256; app boundaries are enforced in SQLite. Tokens and secrets can be piped via stdin to prevent leaking into process listings (`ps`, `/proc`) or shell history.
* **Zero-SSH Policy & SPKI Pinning:** No private SSH keys stored in developer environments or agent contexts. All communication routes through `https://<server-ip>:8443` pinned to the server's immutable SPKI key (`sha256:...`).
* **Encrypted Secrets & Cleaned Envs:** Secrets are encrypted with ChaCha20-Poly1305 on the node, injected via ephemeral `0600` env files unlinked immediately, and masked as `[REDACTED]` in logs. Client `.skoci/env` (or legacy `.env.skoci`) is restricted to an allowlist (`SKOCI_SERVER_URL`, `SKOCI_TOKEN`), and spawned subprocesses run with sanitized environments.
* **Packaging Deny List:** `skoci deploy` excludes sensitive files (`.env*`, `*.pem`, `*.key`, `id_*`, `.npmrc`, `.aws/`, `.ssh/`) and honours `.skoci/ignore` (or `.dockerignore`), and uploads use private `0600` temporary archives.
* **Local Control Plane & Pinned TLS Bind:** API binds to `127.0.0.1:8787` locally and `0.0.0.0:8443` with SPKI-pinned TLS. On public HTTPS (port 443), requests enforce strict TLS SNI matching against `:authority`/`Host` headers (rejecting mismatches with `421 Misdirected Request`).
* **Host Ownership Protection:** Custom domains and auto hosts are strictly partitioned; cross-app host collisions or hijacking are rejected.
* **Human Approval Gates & Image ID Pinning:** Production releases, database restorations, and destructive operations enforce human sign-off pinned to immutable Docker image IDs (preventing TOCTOU tampering).
* **Systemd Hardening:** Service runs under a sandboxed unit with `UMask=0077`, `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`, `PrivateTmp=true`, `PrivateDevices=true`, `ProtectKernelTunables=true`, `ProtectKernelModules=true`, `ProtectControlGroups=true`, `RestrictSUIDSGID=true`, `LockPersonality=true`, `RestrictNamespaces=true`, and `LimitNOFILE=65536`.
* **Audit Log & Build Provenance:** Mutating operations are recorded in an append-only audit log. Official release binaries provide GitHub Actions build provenance attestations (`gh attestation verify`).

### Residual Risks

* **Docker Group ≈ Root:** Access to `/var/run/docker.sock` is root-equivalent on Linux; Skoci hardens process execution, but Docker socket privileges remain root-level.
* **Unrestricted Container Outbound Network:** Containers execute user/agent code inside standard Docker sandboxes with unrestricted outbound internet connectivity.
* **First-Use Trust Established Over SSH (TOFU):** Initial server setup (`skoci server setup`, Skoci >= 0.4.0) provisions the VPS and establishes the SPKI pin over SSH. If the initial SSH connection is intercepted without verifying SSH host keys, the initial pin trust could be compromised. Future connections are pinned in `~/.config/skoci/servers.json`.
* **Single-Server Docker Isolation:** Isolation relies on Linux kernel namespaces and cgroups (Docker defaults) without hardware microVM isolation.
* **First-Come Custom Domains:** An agent can claim any custom domain not yet registered by another application on the instance.
* **Same-Origin Release Checksums:** Release checksums are hosted alongside binary assets; verify provenance with `gh attestation verify skoci-linux-x86_64 --repo imamjabar/skoci`.

---

## Upgrading

### To 0.4.1 and later
Upgrade the server and every workstation together; the client refuses to talk to a server on a different minor version.
```bash
curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash
```
Existing projects keep working untouched: a root `skoci.yaml`, `Dockerfile` and `.env.skoci` are still read. To adopt the `.skoci/` layout in a project, delete the generated root files and run `skoci init` again — keep a `Dockerfile` you wrote yourself, it stays at the root either way.

### From 0.1

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
4. **Reconnect & Update Config:** Reconnect your workstation (`skoci connect`) and re-set secrets (`skoci secret set`). Update `.skoci/skoci.yaml` (or legacy `skoci.yaml`): `env.public` is now a map (`KEY: value`), `build.mode` and `server` are obsolete, and `skoci.policy.yaml` can be deleted.

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
