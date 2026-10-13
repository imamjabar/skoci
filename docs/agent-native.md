# AI Coding Agents as Primary Operators (Agent-Native Architecture)

Traditional deployment platforms (like Coolify and Dokploy) are designed for humans clicking through web forms, treating AI agents as external add-ons.

**Skoci is designed from day one around the principle: AI agents deploy and monitor, humans validate and approve.**

---

## 1. MCP (Model Context Protocol) Integration

Skoci ships with a built-in MCP server that communicates over JSON-RPC 2.0 via standard input/output (`stdio`).

### A. Claude Code Configuration (`.mcp.json` or `~/.claude.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "https://<server-ip>:8443",
        "SKOCI_SERVER_PIN": "sha256:<spki-pin>",
        "SKOCI_TOKEN": "sk_agent_toko_xxxxxxxxxxxx"
      }
    }
  }
}
```

### B. OMP Configuration (`~/.omp/agent/config.yml` or `.omp/mcp.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "https://<server-ip>:8443",
        "SKOCI_SERVER_PIN": "sha256:<spki-pin>",
        "SKOCI_TOKEN": "sk_agent_toko_xxxxxxxxxxxx"
      }
    }
  }
}
```

### C. Zero-Config Mode (Auto-Detect via Profile & `.skoci/env`)
If you have run `skoci connect` or `skoci server setup` (Skoci >= 0.4.0), credentials and pins stored in `~/.config/skoci/servers.json` (`0600`, default store) or project-local `.skoci/env` (`0600`, written when `--project-env` is passed; legacy `.env.skoci` is migrated automatically) are loaded automatically by `skoci mcp` without requiring manual environment variables.
---

## 2. Available MCP Tools

All tools are derived from a single API source of truth (`skoci-api`), guaranteeing 100% parity with the CLI:

| Tool | Parameters | What It Does |
| :--- | :--- | :--- |
| `skoci_init` | `path`, `app` | Detect project framework and generate .skoci/skoci.yaml, .skoci/Dockerfile, and .skoci/ignore |
| `skoci_deploy` | `path`, `env` | Deploy project to Skoci server (pack, upload, and stream events until complete) |
| `skoci_watch` | `deployment_id` | Watch a running deployment until completion and return final status |
| `skoci_status` | `app`, `path` | Check container status, live health, and application URLs |
| `skoci_logs` | `app`, `env`, `tail`, `path` | View recent application logs with error line summary |
| `skoci_deployments` | `app`, `limit`, `path` | List application deployment history |
| `skoci_secret_set` | `app`, `key`, `value`, `path` | Set encrypted secret for application |
| `skoci_secret_list` | `app`, `path` | List application secret key names (values remain encrypted and hidden) |
| `skoci_promote` | `app`, `reason`, `version`, `path` | Submit promotion from preview to production (first promotion goes live immediately by default; subsequent promotions, or all under `--strict-approvals`, require admin approval) |
| `skoci_rollback` | `app`, `env`, `target_version`, `path` | Roll back application to a previous healthy version |
| `skoci_approvals` | *(none)* | List pending approval requests waiting for admin action |
| `skoci_doctor` | *(none)* | Check health of Skoci server, Docker daemon, RAM, and proxy status |
| `skoci_db_backup` | `app`, `service`, `path` | Create secure application database backup |
| `skoci_db_list` | `app`, `path` | List database backup history for application |
| `skoci_db_restore` | `app`, `backup_id`, `service`, `path` | Submit database restore from backup file (requires admin approval when called by agent) |

---

## 3. Tool Safety Boundaries (Strict Human Exclusion)

Certain actions are **deliberately absent from the MCP tool definition** to preserve human-in-the-loop security:

1. **Approving a promotion, historical rollback, or database restore (`skoci approve` / `/api/approve`):** Agents can request promotion to production (`skoci_promote`), database restoration (`skoci_db_restore`), or rollbacks (`skoci rollback`).
   - **Master Token:** Promotes or rolls back directly on the server without requiring approval.
   - **Scoped Agent Token (`sk_agent_*`):**
     - **Promotions:** An application's **first** promotion to production succeeds immediately by default because no live production traffic exists yet. Every **subsequent** promotion (and every database restore or app deletion) produces an approval request with status `pending_approval` and exits with code 2. A server configured with `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` (`skoci server setup --strict-approvals`) gates the first promotion too.
     - **Rollbacks:** Rolling production back to the immediately preceding version (last known-good) is allowed without approval. Rolling back to any older historical version files an approval request (`appr_…`, exit code 2, action `rollback`). When `--strict-approvals` is active, every production rollback requires human approval. Preview rollbacks are never gated.
   - **No Auto-Approve:** The agent **CANNOT** approve its own request, and there is no auto-approve flag.
   - **MCP Tool Restrictions:** MCP intentionally exposes only the listing tool (`skoci_approvals`), not approve or reject tools.
   - **Human Escalation:** When exit code 2 is returned, the agent must extract and surface `Request ID: appr_…` to the human operator, who approves via the Web Dashboard at `https://<server-ip>:8443` or on the server via `sudo skoci approve <request_id>` (server-side admin commands require `sudo` because `/etc/skoci/skoci.env` is `0600 root:root`).
   - If an agent attempts to invoke `/api/approve` with a scoped token, the system rejects it with an `unauthorized` error.
2. **Upgrading the server:** There is no `skoci server upgrade` CLI command. Upgrades of the server binary and background systemd service are host-level operations performed deliberately by a human operator re-running the installer on the server (`curl -fsSL https://raw.githubusercontent.com/imamjabar/skoci/main/install.sh | bash -s --` with any original server flags, or `--version <tag>`). Because Skoci enforces a minor-version lock between CLI and daemon (a 0.6.x CLI refuses to communicate with a 0.5.x server), the operator must upgrade the server first, then the client. Upgrades must not be triggered automatically by an AI agent during an application deployment session.
---

## 4. Standard Exit Codes & Deployment Result States

Skoci returns consistent exit codes for agent automation:

| Exit Code | Name | Description | Agent Action |
| :---: | :--- | :--- | :--- |
| `0` | `Success` | Command completed successfully (e.g. `skoci deploy` -> `live`, `skoci validate` -> valid). | Proceed to next workflow step. |
| `1` | `General Error` | Build failure, health check timeout, or invalid config (`skoci validate` / `skoci init`). | Inspect the `fix` field in the JSON error and remediate. |
| `2` | `Approval Pending` | Action requires human approval (subsequent `skoci promote` with scoped token, or first under `--strict-approvals`; historical production `skoci rollback` older than last known-good; `db restore`; `app delete`). | Extract `Request ID: appr_…` and surface to human operator for `sudo skoci approve <id>`. |
| `3` | `Auth Error` | `unauthorized`, `scope_violation`, or `rate_limited`. | Check token, pin, and application scope permissions. |

### Handling `degraded` Deployment Results
When `skoci deploy --json` completes, inspect `data.status`:
* `live`: Container started, health checks passed, and smoke test (`GET /`) returned 2xx/3xx/401. Deployment is fully healthy.
* `degraded`: Container started and passed health checks, but smoke test (`GET /`) failed (e.g. 502, 500, or connection refused).
  - **Automatic Revert Behavior:** When a smoke test fails on an app that has an existing live route, Skoci automatically reverts the route to the previous container in both proxy router and SQLite database, stops and removes the broken container, and leaves the previous version serving traffic. The deployment status completes as `degraded` pointing at `skoci logs <app>`. If the app had no previous route, the new container stays live as `degraded` so the operator can inspect it without taking the app offline.
  - **Pipeline Cleanup Guards:** Unusable containers (host-port lookup failure), failed health checks (container and its unreferenced image), and cutover failures are cleaned up immediately; image pruning also runs after failed deploys.
  - **Agent Action:** **Treat `degraded` as a failure to investigate.** Do NOT report success to the user and do NOT attempt to promote a degraded version to production. Run `skoci logs --tail 100 --json` to diagnose the error, fix the root route or listening address, and redeploy.
* `failed`: Compilation or startup failure. Inspect `error.fix` and resolve.

### Per-App Concurrency Lock
Deploy, promote, rollback, and delete operations on the same app are mutually exclusive. A concurrent operation returns error code `conflict` (exit code 1 or HTTP 409) naming the running deployment or deletion ID (`Wait for the current deployment to finish before starting a new one`).
---

## 5. Error Handling: `{code, message, fix}`

Whenever an error occurs, Skoci returns structured JSON that can be consumed directly by LLMs:

```json
{
  "success": false,
  "error": {
    "code": "secret_missing",
    "message": "Secret 'DATABASE_URL' is not set",
    "fix": "skoci secret set DATABASE_URL <value>"
  },
  "summary": "secret_missing: Secret 'DATABASE_URL' is not set"
}
```

The agent reads the `fix` field and executes the suggested fix directly without guessing.

---

## 6. Core Rules for Coding Agents

1. **Always use `--json`:** Ensure outputs are structured and machine-parsable.
2. **Connect with Pin & Zero SSH:** Use `skoci connect --server <ip> --token <token> --pin <pin>` (saves to `~/.config/skoci/servers.json` `0600` by default; pass `--project-env` only when `.skoci/env` is needed for CI). After initial setup, never request or attempt SSH operations.
3. **Validate Config & Upload Set Offline First:** Run `skoci validate --verbose --json` to verify `.skoci/skoci.yaml`, required secrets, domain routing, and the exact file list that `skoci deploy` will upload.
4. **Understand Manifest App Naming:** `skoci init` determines default application names from `package.json` (`@scope/` stripped), `Cargo.toml`, or `pyproject.toml` before falling back to directory names, validating format upfront.
5. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`). Verify domain DNS before deploying with `skoci domain check <host>`. (Operators can override this for demos via `skoci server setup --allow-magic-dns-production` or `SKOCI_ALLOW_MAGIC_DNS_PRODUCTION=true`).
6. **Treat `degraded` as an Issue:** When a deploy reports `status: "degraded"`, do not promote. Read `skoci logs --tail 100 --json`, remediate the failure, and deploy again.
7. **Human Gate & Approval Protocol:** Never attempt to approve promotions or rollbacks. With a scoped agent token, an app's first promotion goes live immediately by default (`--strict-approvals` gates the first one too), while subsequent promotions and historical rollbacks (older than last known-good) exit 2 with `pending_approval`; extract `Request ID: appr_…` and ask the human operator to approve via `sudo skoci approve <id>` on the server or in the web dashboard.
8. **Never print tokens & Rely on Log Redaction:** Do not output secret tokens into chat context or repository files. Application and build logs automatically mask registered secrets, Skoci tokens (`sk_agent_…`, `sk_master_…`), and `Authorization: Bearer …` headers as `[REDACTED]`.
9. **Follow `fix` guidance:** When an error occurs, inspect and execute the remedy in `error.fix`.
