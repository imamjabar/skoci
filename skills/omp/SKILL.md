---
name: skoci
description: Deploy, monitor, and manage containerized services on VPS using the lightweight Skoci platform.
---

# Skoci Harness Skill for OMP

Use this skill when deploying or managing applications via Skoci.

## Rules
1. **Machine output:** Always run commands with `--json` for structured, machine-parsable JSON outputs.
2. **Follow error `fix`:** In case of error, parse `{ "code": "...", "message": "...", "fix": "..." }` and execute the suggested `fix` instruction.
3. **Exit codes & Result States:**
   - `0`: Success (`live`).
   - `1`: Operation failed (compilation error, health check timeout, bad configuration in `skoci validate` or `init`).
   - `2`: Approval pending/required (production promotion, database restore, app deletion).
   - `3`: Authentication/TLS error (`unauthorized`, `scope_violation`, `pin_required`, `pin_mismatch`, `rate_limited`).
   - **`degraded` Result State:** If deployment returns `status: "degraded"` (container is running and health probe passed, but smoke test `GET /` did not return 2xx/3xx/401), **treat it as a failure to investigate**. Do NOT report success and do NOT promote to production. Run `skoci logs --tail 100 --json` to diagnose the error, fix the root route or listening address, and redeploy.
4. **Human gate approval:** Never attempt to approve production deployments or restores directly (agents cannot self-approve). Master token promotes directly; scoped agent token (`sk_agent_*`) always returns status `pending_approval` (exit code 2) with a `request_id` (`appr_…`). Inform the human operator to approve on the dashboard (`https://<server-ip>:8443`) or via `skoci approve <request_id>`.
5. **Zero SSH & SPKI Pinning:** Do not use SSH for normal operations. Connect using `skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json` which persists to global profiles `~/.config/skoci/servers.json` and local `.skoci/env` (or legacy `.env.skoci`).
6. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`). Always verify domain DNS records before deploying with `skoci domain check <host> --json`.
7. **Token privacy:** Never print, log, or store plain-text tokens in logs or git repositories.
8. **Remote builds:** All Docker container builds run on the remote Skoci server; Docker is not required on the local workstation.
## Common Sequences
```bash
# Connect workspace to server with SPKI certificate pin
skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json

# Verify custom domain DNS record before deploying
skoci domain check <host> --json
# Initialize project (infers app name from manifest, non-root Dockerfile, strips .skoci/)
skoci init --json

# Offline configuration validation (zero network, exit 0 valid, 1 invalid)
skoci validate --json

# Set secret environment variables (ChaCha20 encrypted on server)
skoci secret set API_KEY "super-secret" --json

# Deploy to preview environment (streams build & health events)
skoci deploy --env preview --json

# Check container status & health probe
skoci status --json

# View container logs with automatic secret redaction
skoci logs --tail 100 --json

# Request production promotion
skoci promote "Ready for v1 release" --json

# Rollback if an issue is detected
skoci rollback --json

# On-demand database backup
skoci db backup --json

# Run diagnostics
skoci doctor --json
```
