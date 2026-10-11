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
If you have run `skoci connect` or `skoci server setup` (Skoci >= 0.4.0), credentials and pins stored in `~/.config/skoci/servers.json` or project-local `.skoci/env` (or legacy `.env.skoci`) are loaded automatically by `skoci mcp` without requiring manual environment variables.
---

## 2. Available MCP Tools

All tools are derived from a single API source of truth (`skoci-api`), guaranteeing 100% parity with the CLI:

| Tool Name | Purpose | Parameters |
| :--- | :--- | :--- |
| `skoci_init` | Detect framework & create `.skoci/skoci.yaml` | `app`, `path` |
| `skoci_deploy` | Upload, compile image, & cutover | `app`, `env` (`preview`/`production`), `commit` |
| `skoci_status` | Inspect container health & status | `app` |
| `skoci_logs` | Retrieve recent logs (secrets redacted) | `app`, `tail` |
| `skoci_secret_set` | Store ChaCha20 encrypted secret | `app`, `key`, `value` |
| `skoci_secret_list` | List secret key names | `app` |
| `skoci_promote` | Request production promotion approval | `app`, `reason` |
| `skoci_rollback` | Revert to previous working version | `app`, `target_version` |
| `skoci_doctor` | Inspect idle RAM (<20 MB) & daemons | *(none)* |
| `skoci_db_backup` | Trigger database backup snapshot | `app`, `service` |
| `skoci_db_restore` | Request database restore (requires approval) | `app`, `service`, `backup_id` |

---

## 3. Tool Safety Boundaries (Strict Human Exclusion)

The approval action (`approve`) is **intentionally EXCLUDED from the agent tool definition**:
1. Agents **CANNOT** approve their own requests. Production promotions and restores strictly require human operator authorization via the Dashboard or master token.
2. If an agent attempts to invoke the approval endpoint (`/api/approve`), the system rejects the request with an `unauthorized` error.

---

## 4. Standard Exit Codes & Deployment Result States

Skoci returns consistent exit codes for agent automation:

| Exit Code | Name | Description | Agent Action |
| :---: | :--- | :--- | :--- |
| `0` | `Success` | Command completed successfully. | Proceed to next workflow step. |
| `1` | `General Error` | Build failure, health check timeout, or invalid configuration. | Inspect the `fix` field in the JSON error and remediate. |
| `2` | `Approval Pending` | Action requires human approval (`promote`, `db restore`). | Note the `request_id` and ask human to approve in the dashboard. |
| `3` | `Auth Error` | `unauthorized`, `scope_violation`, or `rate_limited`. | Check token, pin, and application scope permissions. |

### Handling `degraded` Deployment Results
When `skoci deploy --json` completes, inspect `data.status`:
* `live`: Container started, health checks passed, and smoke test (`GET /`) returned 2xx/3xx/401. Deployment is fully healthy.
* `degraded`: Container started and passed health checks, but smoke test (`GET /`) failed (e.g. 502, 500, or connection refused).
  - **Agent Action:** **Treat `degraded` as a failure to investigate.** Do NOT report success to the user and do NOT attempt to promote a degraded version to production. Run `skoci logs --tail 100 --json` to diagnose the error, fix the root route or listening address, and redeploy.
* `failed`: Compilation or startup failure. Inspect `error.fix` and resolve.
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
2. **Connect with Pin & Zero SSH:** Use `skoci connect --server <ip> --token <token> --pin <pin>` (or saved profiles). After initial setup, never request or attempt SSH operations.
3. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`). Verify domain DNS before deploying with `skoci domain check <host>`.
4. **Treat `degraded` as an Issue:** When a deploy reports `status: "degraded"`, do not promote. Read `skoci logs --tail 100 --json`, remediate the failure, and deploy again.
5. **Never print tokens:** Do not output secret tokens into chat context or repository files.
6. **Follow `fix` guidance:** When an error occurs, inspect and execute the remedy in `error.fix`.
