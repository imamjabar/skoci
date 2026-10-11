---
name: skoci
description: Operator skill for deploying, monitoring, and managing applications on Skoci.
---

# Skoci Skill for Claude Code

Operate Skoci deployment platform via the CLI or MCP tools.

## Core Rules for Agents
1. **Always use `--json`:** Run CLI commands with `--json` for machine-readable output.
2. **Follow `fix` on errors:** In case of failure, check the `error.code`, read the `error.fix` field, and execute that fix.
3. **Exit codes:**
   - `0`: Success.
   - `1`: General failure (build failed, health check failed, config invalid).
   - `2`: Approval pending/required (production deployment, database restore, deletion).
   - `3`: Authentication error (`unauthorized`, `scope_violation`, `rate_limited`).
4. **Approval protocol:**
   - Production deployments (`skoci promote "<reason>" --json`) return exit code 2 and a `request_id`.
   - Never attempt to approve directly (agents lack permission). Inform the human operator to approve via the Web Dashboard at `https://skoci.<base-domain>` or via `skoci approve <request_id>`.
5. **Zero SSH:** Never request or run SSH commands after server installation. Use `skoci connect --server <url> --token <token>`.
6. **Token security:** Never echo or print tokens (`sk_agent_*`, `sk_master_*`) into conversations or commit files.
7. **No local Docker:** Builds always occur on the remote server; the local environment does not require Docker.

## Common CLI Commands
```bash
# Connect workspace to Skoci server
skoci connect --server https://skoci.<base-domain> --token sk_agent_<app>_<hex> --json

# Initialize project configuration (skoci.yaml, Dockerfile)
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
    "message": "Secret 'DATABASE_URL' belum diset",
    "fix": "skoci secret set DATABASE_URL <value>"
  },
  "summary": "secret_missing: Secret 'DATABASE_URL' belum diset"
}
```
Read the `fix` string and execute it directly.
