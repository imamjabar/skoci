---
name: skoci
description: Operator skill for deploying, monitoring, and managing applications on Skoci.
---

# Skoci Skill for Claude Code

Operate Skoci deployment platform via the CLI or MCP tools.

## Core Rules for Agents
1. **Always use `--json`:** Run CLI commands with `--json` for machine-readable output.
2. **Follow `fix` on errors:** In case of failure, check the `error.code`, read the `error.fix` field, and execute that fix.
3. **Exit codes & Result States:**
   - `0`: Success (`live`).
   - `1`: General failure (build failed, health check failed, config invalid).
   - `2`: Approval pending/required (production deployment, database restore, deletion).
   - `3`: Authentication/TLS error (`unauthorized`, `scope_violation`, `pin_required`, `pin_mismatch`, `rate_limited`).
   - **`degraded` Result State:** If deployment finishes with `status: "degraded"` (container is running and passed health check, but smoke test `GET /` did not return 2xx/3xx/401), **treat it as a failure to investigate**. Do NOT report success and do NOT promote to production. Run `skoci logs --tail 100 --json` to diagnose why the root route failed, fix the code/routing/port configuration, and redeploy.
4. **Approval protocol:**
   - Production deployments (`skoci promote "<reason>" --json`) return exit code 2 and a `request_id`.
   - Never attempt to approve directly (agents lack permission). Inform the human operator to approve via the Web Dashboard at `https://<server-ip>:8443` or via `skoci approve <request_id>`.
5. **Zero SSH & SPKI Pinning:** Never request or run SSH commands after initial server installation. Connect using `skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json`.
6. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`). Always verify domain DNS records before deploying with `skoci domain check <host> --json`.
7. **Token security:** Never echo or print tokens (`sk_agent_*`, `sk_master_*`) into conversations or commit files.
8. **No local Docker:** Builds always occur on the remote server; the local environment does not require Docker.
## Common CLI Commands
```bash
# Connect workspace to Skoci server with SPKI certificate pin
skoci connect --server <server-ip> --token sk_agent_<app>_<hex> --pin sha256:<spki-pin> --json

# Verify custom domain DNS record before deploying
skoci domain check <host> --json

# Initialize project configuration (.skoci/skoci.yaml, .skoci/Dockerfile, .skoci/ignore)
skoci init --json

# Set secret environment variables (encrypted at rest on server)
skoci secret set API_KEY "value" --json

# Deploy to preview environment
skoci deploy --env preview --json

# Check live container health & status
skoci status --json

# Inspect container logs (secrets are automatically redacted)
skoci logs --tail 100 --json

# Request production promotion (Human Gate)
skoci promote "Ready for production release" --json

# Check pending approvals
skoci approvals --json

# Rollback to previous working version
skoci rollback --json

# Create on-demand database backup
skoci db backup --json

# Diagnostic health check
skoci doctor --json
```

## Error Recovery Protocol
When an API error occurs, the JSON response structure is:
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
Read the `fix` string and execute it directly.
