# Deploy Your First Application on Skoci

A step-by-step guide to deploying a web application to your server using Skoci.

---

## 1. Connect Workspace to Skoci Server

Before deploying, link your local repository to your Skoci server using the application token created on the server:

```bash
skoci connect --server https://skoci.203-0-113-10.sslip.io --token sk_agent_toko_xxxxxxxxxxxx
```

This validates the server connection and securely stores credentials in `.env.skoci` (file permissions `0600`, automatically added to `.gitignore`).

---

## 2. Initialize the Project (`skoci init`)

Run the initialization command inside your application repository:

```bash
cd ~/projects/toko
skoci init
```

Skoci automatically detects your application framework:
* **Static HTML:** presence of `index.html` → Nginx alpine, port 80, automatic `/healthz` endpoint.
* **Node.js with `start` script:** `package.json` → Dockerfile `node:22-alpine`, port 3000.
* **Frontend build (Vite/React without `start`):** built with Node, then served by Nginx, port 80.
* **Python / FastAPI / Flask / Django:** `requirements.txt` or `pyproject.toml` → port 8000.
* **Go:** `go.mod` → static binary `CGO_ENABLED=0`, port 8080.
* **Rust:** `Cargo.toml` → multi-stage cargo build, port 8080.
* **PHP / Laravel:** `composer.json` → `php:8.3-apache`, port 80.
* **Docker:** existing `Dockerfile` used as-is.

Databases are only added to `services` when matching dependencies are detected (e.g. `pg`, `psycopg`, `mysql2`, `redis`). `skoci init` prints recommendations (such as chosen health check endpoints) — review them before deploying.

Initialization creates:
1. `skoci.yaml`: Declarative contract for your application.
2. `Dockerfile`: Container image recipe (if not already present).
3. `.dockerignore`: Prevents sensitive directories (`.git`, `.env.skoci`) from being uploaded.

### Why Your Laptop Does Not Need Docker
The Dockerfile is just a build recipe. Container compilation runs entirely on your remote VPS. Your local machine or AI agent sandbox only archives the project files and uploads them over HTTPS.

---

## 3. Customizing `skoci.yaml`

The `skoci.yaml` file uses a structured schema:

```yaml
version: 1
app: toko
build:
  dockerfile: Dockerfile
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
* In `skoci.yaml`: declare key names (`API_KEY`).
* Secret values are sent directly to the Skoci server and stored encrypted:

```bash
skoci secret set API_KEY "production-secret-key"
```

Secrets are encrypted with **ChaCha20-Poly1305** using the node key (`/var/lib/skoci/node.key`, generated at installation). Secret values **are never returned to AI agents**, injected via temporary env-files (permissions `0600`, deleted immediately), and redacted in logs.

---

## 5. Deploying to Preview

Deploy your application to the preview environment:

```bash
skoci deploy --env preview
```

### Skoci Deployment Pipeline Stages:
1. **Packaging & Upload:** Project files are packed respecting `.gitignore` and uploaded over HTTPS.
2. **Extract:** Server extracts the archive and validates `skoci.yaml`.
3. **Build:** Server builds the Docker image `skoci/<app>:<version>` with live streaming logs.
4. **Services:** Server ensures database containers (if declared) are healthy and ready.
5. **Start:** The new container starts with secrets injected via a secure temporary env-file.
6. **Health Check:** Reverse proxy polls `run.health` until the container returns HTTP 2xx/3xx.
7. **Cutover:** Reverse proxy atomically swaps routing targets without downtime.
8. **Smoke Test:** HTTP GET check verifies the new container is serving responses.
9. **Cleanup:** Old container is stopped and removed after a 5-second drain window.

Your preview application is immediately accessible at:
`https://toko-preview.203-0-113-10.sslip.io` (or your configured preview domain).

*Note: accessing the server IP directly (`http://203.0.113.10`) serves the **default app** (the first app deployed; change via `skoci default-app <app>` using a master token). This IP URL is printed in deploy output if your app is the default app.*

---

## 6. Using Custom Domains

To use your own domain:
1. Point DNS **A Records** to your server public IP:
   - `toko.sumopod.id` → `SERVER_IP`
   - `toko-preview.sumopod.id` → `SERVER_IP`
2. Add them to `skoci.yaml`:
   ```yaml
   domains:
     - toko.sumopod.id
   preview_domains:
     - toko-preview.sumopod.id
   ```
3. Run `skoci deploy --env preview` or `skoci deploy --env production`. Skoci's reverse proxy automatically provisions TLS certificates from Let's Encrypt!

---

## 7. Inspecting Status & Logs

Check container health in real-time:
```bash
skoci status
```

Inspect container output logs (secrets are masked as `[REDACTED]`):
```bash
skoci logs --tail 50
```

---

## 8. Promoting to Production (Human Gate)

By default, shipping to production requires human approval:

```bash
skoci promote "All preview tests passed, ready for production"
```

The command returns exit code 2 (*pending approval*) and a `request_id`, e.g. `appr_4e052b4e...`.

A human operator approves it via:
* **Web Dashboard:** Open `https://skoci.<base-domain>`, log in with the master token, and click **Approve**.
* **Server CLI:**
  ```bash
  sudo skoci approve appr_4e052b4e...
  ```

Once approved, the production container becomes active and traffic is routed to `https://toko.<base-domain>`.

---

## 9. 1-Click Rollback

If an issue is detected, rollback to the previous version:

```bash
skoci rollback                 # rollback preview to previous version
skoci rollback --env production --to v20261011031108
```

Skoci relaunches the previous image without rebuilding, waits for health checks to pass, and swaps traffic. The current container keeps serving traffic until the rollback container is confirmed healthy.
