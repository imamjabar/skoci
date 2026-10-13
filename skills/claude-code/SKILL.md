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
   - `1`: General failure (build failed, health check failed, config invalid in `skoci validate` or `skoci init`).
   - `2`: Approval pending/required (subsequent production promotions via scoped token, or first when `--strict-approvals` is active; production rollback to historical versions older than last known-good; database restore; deletion).
   - `3`: Authentication/TLS error (`unauthorized`, `scope_violation`, `pin_required`, `pin_mismatch`, `rate_limited`).
   - **`degraded` Result State:** If deployment finishes with `status: "degraded"` (container passed health check, but smoke test `GET /` failed; traffic automatically reverts to previous container and the broken container is removed), **treat it as a failure to investigate**. Do NOT report success and do NOT promote to production. Run `skoci logs --tail 100 --json` to diagnose why the root route failed, fix the code/routing/port configuration, and redeploy.
4. **Approval protocol:**
   - Master token on server promotes and rolls back directly without approval.
   - With a scoped agent token (`sk_agent_*`), an app's **first** promotion to production goes live immediately by default (unless `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` / `--strict-approvals` is enabled on the server); every **subsequent** promotion returns exit code 2, status `pending_approval`, and prints `Request ID: appr_…`.
   - Rolling production back to the immediately preceding version (last known-good) is allowed without approval. Rolling back to older historical versions produces an approval request (exit code 2, action `rollback`). When `--strict-approvals` is active, all production rollbacks require human approval. Preview rollbacks are ungated.
   - Never attempt to approve directly (agents lack permission, no auto-approve exists). Inform the human operator to approve via the Web Dashboard at `https://<server-ip>:8443` or on the server via `sudo skoci approve <request_id>`.
5. **Zero SSH & SPKI Pinning:** Never request or run SSH commands after initial server installation. Connect using `skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json` (saves to `~/.config/skoci/servers.json` `0600` by default; pass `--project-env` only when `.skoci/env` is needed for CI).
6. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`) unless the server operator enabled `--allow-magic-dns-production`. Always verify custom domain DNS records before deploying with `skoci domain check <host> --json`.
7. **Token security & Log Redaction:** Never echo or print tokens (`sk_agent_*`, `sk_master_*`) into conversations or commit files. Application and build logs automatically mask registered secrets, Skoci tokens, and `Authorization: Bearer …` headers as `[REDACTED]`.
8. **Concurrency:** Deploy, promote, rollback, and delete operations on the same app are mutually exclusive; concurrent actions return `conflict` error naming the active deployment/deletion ID.
9. **No local Docker:** Builds always occur on the remote server; the local environment does not require Docker.
## Common CLI Commands
```bash
# Connect workspace to Skoci server with SPKI certificate pin
skoci connect --server <server-ip> --token sk_agent_<app>_<hex> --pin sha256:<spki-pin> --json

# Verify custom domain DNS record before deploying
skoci domain check <host> --json

# Initialize project configuration (.skoci/skoci.yaml, non-root .skoci/Dockerfile, .skoci/ignore)
# Static sites use nginxinc/nginx-unprivileged:1.27-alpine on port 8080 and strip .skoci/
skoci init --json

# Validate project configuration and inspect exact upload file list offline
skoci validate --verbose --json

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
