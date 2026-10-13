# Deploy Your First Application on Skoci

A step-by-step guide to deploying a web application to your server using Skoci.

---

## 1. Connect Workspace to Skoci Server

Link your local repository to your Skoci server using the server IP, application token, and SPKI certificate pin:

```bash
skoci connect --server <server-public-ip> --token sk_agent_toko_xxxxxxxxxxxx --pin sha256:7b5fa1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcd
```

The control plane is reachable at `https://<public-ip>:8443` over TLS pinned to the server's SPKI public key (`sha256:...`). Connections do not depend on third-party wildcard DNS.

* **Global Server Profiles (Default):** `skoci connect` saves connection profiles in `~/.config/skoci/servers.json` (permissions `0600`) by default.
* **Project Override (`--project-env`):** Pass `skoci connect --project-env` when you also want a repository-local `.skoci/env` file (permissions `0600`, automatically added to `.gitignore`) for CI or per-repo overrides. An existing `.skoci/env` is always read when present, and a legacy `.env.skoci` is automatically migrated to `.skoci/env`.
---

## 2. Initialize the Project (`skoci init`)

Run the initialization command inside your application repository:

```bash
cd ~/projects/toko
skoci init
```

Skoci automatically detects your application framework:
* **Static HTML:** presence of `index.html` → `nginxinc/nginx-unprivileged:1.27-alpine` (`USER nginx`), port 8080, automatic `/healthz` endpoint, internal-file `404` rules.
* **Node.js with `start` script:** `package.json` → Dockerfile `node:22-alpine`, port 3000.
* **Frontend build (Vite/React without `start`):** built with Node (`node:22-alpine`), then served by `nginxinc/nginx-unprivileged:1.27-alpine` (`USER nginx`), port 8080.
* **Python / FastAPI / Flask / Django:** `requirements.txt` or `pyproject.toml` → port 8000.
* **Go:** `go.mod` → static binary `CGO_ENABLED=0`, port 8080.
* **Rust:** `Cargo.toml` → multi-stage cargo build, port 8080.
* **PHP / Laravel:** `composer.json` → `php:8.3-apache`, port 80.
* **Docker:** existing `Dockerfile` used as-is.

Databases are only added to `services` when matching dependencies are detected (e.g. `pg`, `psycopg`, `mysql2`, `redis`). `skoci init` prints recommendations (such as chosen health check endpoints) — review them before deploying.

### Default App Name Resolution
`skoci init` determines the default application name from project manifests in order:
1. `package.json`: reads the `name` field, stripping any leading `@scope/` prefix.
2. `Cargo.toml`: reads `[package] name`.
3. `pyproject.toml`: reads `[project] name` or `[tool.poetry] name`.
4. Directory name fallback: used only when no manifest defines an application name.

The candidate name is validated upfront (must be 1–40 lowercase alphanumeric characters and hyphens, start with a letter/digit, contain no consecutive hyphens `--`, no trailing hyphen, not end with `-preview`, and not be reserved). An unusable name fails immediately at `init` with an actionable fix, preventing late `scope_violation` errors during deployment.

### Generated Artifacts & Container Sandboxing
Initialization creates everything inside the `.skoci/` directory:
1. `.skoci/skoci.yaml`: Declarative contract for your application (commit to git). A legacy root `skoci.yaml` is migrated automatically to `.skoci/skoci.yaml` by `init`, `validate`, `deploy`, and `connect`; the loader only reads `.skoci/skoci.yaml`.
2. `.skoci/Dockerfile`: Container image recipe (commit to git; skipped if you already have your own; configured via `build.dockerfile: .skoci/Dockerfile`). Generated Dockerfiles delete `.skoci/` from the built container image (including `/usr/share/nginx/html/.skoci` in both `Static` and `StaticBuild` templates) and run as a non-root user (`node` for Node, `appuser` for Python, Rust, and Go, `nginx` on port `8080` for Static and StaticBuild). The generated nginx config for static sites also returns `404` for dotfiles, internal extensions (`*.md`, `*.py`, `*.sh`, `*.sql`, `*.yaml`, `*.toml`, `*.lock`, `*.log`, `.env`, etc.), and named manifests (`package.json`, `yarn.lock`, `requirements.txt`, `Makefile`, `Dockerfile*`), while keeping data `*.json` servable.
3. `.skoci/ignore`: Explicit upload filter for `skoci deploy` (commit to git). `skoci deploy` applies, in order: built-in defaults (plus the non-overridable sensitive-file deny list), the project-root `.gitignore`, and `.skoci/ignore` (or `.dockerignore` when `.skoci/ignore` is absent). Parent-directory `.gitignore` files, global gitignore, and `.git/info/exclude` are not applied. The `.skoci/env` credential file is never uploaded.
### Why Your Laptop Does Not Need Docker
The generated `.skoci/Dockerfile` is just a build recipe. Container compilation runs entirely on your remote VPS. Your local machine or AI agent sandbox only archives the project files and uploads them over HTTPS.

---

## 3. Customizing `.skoci/skoci.yaml`

The `.skoci/skoci.yaml` file uses a structured schema:

```yaml
version: 1
app: toko
build:
  dockerfile: .skoci/Dockerfile
  context: .
run:
  port: 3000                  # Internal port your app listens on
  health: /healthz            # HTTP health check probe path for zero-downtime cutover
  health_timeout_secs: 60     # Readiness probe timeout (seconds)
  memory: 256m                # Container memory limit (prevents OOM)
domains:
  - toko.sumopod.id           # Custom domain for production
preview_domains:
  - toko-preview.sumopod.id   # Custom domain for preview
env:
  public:
    NODE_ENV: production
  secrets:
    - API_KEY                 # Key name only, never commit values!
```

---

## 4. Storing Encrypted Secrets (`skoci secret set`)

Skoci separates configuration key declarations from sensitive secret values:
* In `.skoci/skoci.yaml`: declare key names (`API_KEY`).
* Secret values are sent directly to the Skoci server and stored encrypted:

```bash
skoci secret set API_KEY "production-secret-key"
```

Secrets are encrypted with **ChaCha20-Poly1305** using the node key (`/var/lib/skoci/node.key`, generated at installation). Secret values **are never returned to AI agents**, injected via temporary env-files (permissions `0600`, deleted immediately), and redacted in logs.


---

## 5. Offline Configuration Validation (`skoci validate`)

Before deploying, validate `.skoci/skoci.yaml` and inspect the upload manifest offline with zero network calls:

```bash
skoci validate
# List exact files that skoci deploy would upload, total size, and excluded paths:
skoci validate --verbose
# Or structured JSON output:
skoci validate --verbose --json
```

`skoci validate [path]` inspects the configuration path, application name, Dockerfile path, build context, declared production and preview domains, and required secrets. Passing `--verbose` (`-v`) also runs the upload filter and returns the exact `files` list, total `bytes`, and `excluded_sensitive` paths that `skoci deploy` would exclude.

* **Exit 0 (Valid):** Configuration is well-formed and passes all rule checks.
* **Exit 1 (Invalid):** Configuration file missing, syntax error, or invalid fields; outputs structured `{code, message, fix}`.
---

## 6. Deploying to Preview

Deploy your application to the preview environment:

```bash
skoci deploy --env preview
```

### Skoci Deployment Pipeline Stages:
1. **Packaging & Upload:** Project files are packed using the explicit filter chain (built-in defaults and sensitive-file deny list, project-root `.gitignore`, then `.skoci/ignore` or `.dockerignore`) and uploaded over HTTPS.
2. **Extract:** Server extracts the archive (preserving executable bits for non-root containers) and validates `.skoci/skoci.yaml`.
3. **Build:** Server builds the Docker image `skoci/<app>:<version>` with live streaming logs.
4. **Services:** Server ensures database containers (if declared) are healthy and ready.
5. **Start:** The new container starts with secrets injected via a secure temporary env-file.
6. **Health Check:** Reverse proxy polls `run.health` until the container returns HTTP 2xx/3xx.
7. **Cutover:** Reverse proxy atomically swaps routing targets without downtime.
8. **Smoke Test:** HTTP GET check (`GET /`) verifies the new container is answering, then polls the public URL until ACME TLS certificate issuance finishes (~7–10 s for a brand-new hostname) so a healthy first deploy completes as `live` without `unready_urls`. If the smoke test fails on an app with an existing route, Skoci automatically reverts traffic to the previous container (router and SQLite), removes the broken new container, and keeps the previous version serving.
9. **Cleanup & Failure Guards:** Old container is stopped and removed after a 5-second drain window. Built-in cleanup guards ensure that unusable containers (host-port lookup failure), failed health checks (container and its unreferenced image), and cutover failures are cleaned up immediately without leaving orphaned resources; image pruning runs after both successful and failed deploys.

### Deployment Result States:
Every deployment finishes in one of three distinct result states:

* **`live` (Success):** The container started, passed the internal health probe (`run.health`), and the reverse proxy smoke test (`GET /`) answered 2xx, 3xx, or 401. Your application is active and serving traffic.
* **`degraded` (Warning):** The container started and passed internal health checks, but the smoke test (`GET /`) did **not** return 2xx/3xx/401 (e.g. returned HTTP 502, 500, or connection refused on the root path).
  - **What It Means:** End-user requests to `/` are failing. If the app had an existing live route, Skoci automatically reverts routing to the previous container, removes the broken container, and keeps the previous version serving without downtime while completing with status `degraded`. If the app had no previous route, the container stays live as `degraded` so the operator can inspect it.
  - **What to Do:** Inspect application logs immediately with `skoci logs --tail 100`. Verify that your web application binds to `0.0.0.0` (not `127.0.0.1`), that required environment variables are set, and that the root route (`/`) is properly handled.
* **`failed` (Error):** Image build failed, container exited unexpectedly during startup, or readiness probes timed out. The previous container (if any) remains live.

### Per-App Concurrency Lock
Deploy, promote, rollback, and delete operations on the same app are mutually exclusive. A concurrent operation returns error code `conflict` (exit code 1 or HTTP 409) naming the running deployment or deletion ID.

Your preview application is immediately accessible at your configured preview domain (e.g. `https://toko-preview.sumopod.id`).

## 7. Domains & DNS Pre-Verification

Production deployments on Skoci require a genuine custom domain. Magic DNS domains (such as `*.sslip.io`) are strictly rejected in production because third-party zones expose deployments to hijacking risks and shared Let's Encrypt rate limits.

### A. Check DNS Record Before Deploying
Before requesting Let's Encrypt certificates, verify your DNS configuration:

```bash
skoci domain check toko.sumopod.id
```

Skoci queries DNS resolvers and displays the exact record required:
```
Domain: toko.sumopod.id
Status: Points to this server (A -> 203.0.113.10)
```
If the record does not point to your server yet, `skoci domain check` prints the exact record hint to create at your DNS provider (e.g. `A toko.sumopod.id -> 203.0.113.10`).

### B. Associate Domain with Application
You can register custom domains via CLI or in `.skoci/skoci.yaml`:
```bash
skoci domain add toko.sumopod.id
```
Or declare them in `.skoci/skoci.yaml`:
```yaml
domains:
  - toko.sumopod.id           # Production custom domain
preview_domains:
  - toko-preview.sumopod.id   # Preview custom domain
```
When deploying, Skoci's reverse proxy automatically provisions TLS certificates from Let's Encrypt via HTTP-01 challenges.

A hostname belongs to exactly one environment. Listing the same host under both `domains` and `preview_domains` is rejected, because the proxy maps one host to one container and the last deploy would silently take over production traffic.

### C. Move the Whole Server to a Domain You Bought
`domains:` attaches one hostname to one app. When you want every app on the server to live under a domain, set the server's base domain (requires the master token on the server, or an admin session):

```bash
skoci domain check sumopod.id
skoci domain set sumopod.id --primary
```

Point `A` records for both `sumopod.id` and `skoci.sumopod.id` at the server first; `--force` skips that precheck. The command persists the base domain, re-points every live route at `<app>.sumopod.id` and `<app>-preview.sumopod.id`, moves the dashboard to `skoci.sumopod.id`, and lets the proxy request the new certificates. No restart, no dropped route, and the setting survives a restart.

## 8. Inspecting Status & Logs

Check container health in real-time:
```bash
skoci status
```

Inspect container output logs (secrets are masked as `[REDACTED]`):
```bash
skoci logs --tail 50
```

---

## 9. Promoting to Production (Human Gate & Approval Flow)

Promote a verified preview deployment to production:

```bash
skoci promote "All preview tests passed, ready for production"
```

The promotion behavior depends on the token and whether the application already has live production traffic:
* **Master Token (Human Admin):** If run on the server with `sudo` (using the master token from `/etc/skoci/skoci.env`), `skoci promote` promotes directly to production without creating a pending approval.
* **Scoped Agent Token (`sk_agent_*`):**
  - **First promotion:** When the application has no live production deployment yet, the first promotion succeeds immediately and goes live (no existing production traffic is at risk).
  - **Subsequent promotions:** Every later promotion returns exit code 2 (*pending approval*), status `pending_approval`, and prints `Request ID: appr_…`. The agent **cannot** approve its own request (there is no auto-approve flag) and must surface the `request_id` to a human operator.
  - **Strict approvals (`--strict-approvals`):** If the server was configured with `skoci server setup --strict-approvals` (or `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` in `/etc/skoci/skoci.env`), the first promotion also requires human approval.

A human operator approves a pending promotion via:
* **Web Dashboard:** Open `https://<server-public-ip>:8443` (or `https://skoci.<base-domain>`) in your browser, log in with the master token, and click **Approve** in the Approvals tab.
* **Server CLI:**
  ```bash
  sudo skoci approve appr_4e052b4e...
  ```
Once approved, the production container becomes active and traffic is routed to `https://toko.<base-domain>`.

### Preview → Production Without a Custom Domain (`sslip.io`)
Magic-DNS hostnames (`sslip.io`) are refused in production by default. For an operator evaluating the full preview → production workflow without a domain:
1. **Enable the server override:** Provision with `skoci server setup --host <server-public-ip> --user root --allow-magic-dns-production` (without `--domain`, magic DNS `<dashed-ip>.sslip.io` is enabled automatically), or set `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true` in `/etc/skoci/skoci.env` on the server and run `sudo systemctl restart skoci`.
2. **Deploy to preview:** `skoci deploy --env preview` builds the container and serves it at `https://<app>-preview.<dashed-ip>.sslip.io`.
3. **Promote to production:** `skoci promote "Initial production release"` goes live immediately at `https://<app>.<dashed-ip>.sslip.io` on the first promotion (unless `--strict-approvals` is enabled). Subsequent promotions return `Request ID: appr_…` (exit code 2), which the operator approves with `sudo skoci approve <request_id>`.

---

## 10. 1-Click Rollback

If an issue is detected, rollback to the previous version:

```bash
skoci rollback                 # rollback preview to previous version
skoci rollback --env production --to v20261011031108
```

Skoci relaunches the previous image without rebuilding, waits for health checks to pass, and swaps traffic. The current container keeps serving traffic until the rollback container is confirmed healthy.

### Rollback Approval Policy
* **Admin / Master Token:** Rollback proceeds immediately without requiring approval.
* **Scoped Agent Token (`sk_agent_*`):**
  - **Last Known-Good Version:** Rolling production back to the version that was live immediately before proceeds without approval.
  - **Older Historical Versions:** Rolling back to any older version files an approval request (`appr_…`, exit code 2, action `rollback`) that a human operator approves via the web dashboard or on the server with `sudo skoci approve <request_id>`.
  - **Strict Approvals (`--strict-approvals`):** If the server was provisioned with `--strict-approvals` or `SKOCI_REQUIRE_APPROVAL_ALWAYS=true`, every production rollback requires human approval.
  - **Preview Rollbacks:** Always ungated.
