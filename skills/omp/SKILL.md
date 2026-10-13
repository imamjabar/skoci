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
   - `2`: Approval pending/required (subsequent production promotions, or first when `--strict-approvals` is active; production rollback to historical versions older than last known-good; database restore; app deletion).
   - `3`: Authentication/TLS error (`unauthorized`, `scope_violation`, `pin_required`, `pin_mismatch`, `rate_limited`).
   - **`degraded` Result State:** If deployment returns `status: "degraded"` (container passed health check, but smoke test `GET /` failed; traffic automatically reverts to previous container and the broken container is removed), **treat it as a failure to investigate**. Do NOT report success and do NOT promote to production. Run `skoci logs --tail 100 --json` to diagnose the error, fix the root route or listening address, and redeploy.
4. **Human gate approval:** Never attempt to approve production deployments, historical rollbacks, or restores directly (agents cannot self-approve). Master token promotes and rolls back directly; with a scoped agent token (`sk_agent_*`), an app's **first** promotion to production goes live immediately by default (unless `SKOCI_REQUIRE_APPROVAL_ALWAYS=true` / `--strict-approvals` is enabled on the server), and every **subsequent** promotion returns status `pending_approval` (exit code 2) with a `request_id` (`appr_…`). Rolling production back to the immediately previous version (last known-good) is allowed without approval; rolling back to older versions requires approval (exit code 2, action `rollback`). When `--strict-approvals` is active, all production rollbacks require approval. Inform the human operator to approve on the dashboard (`https://<server-ip>:8443`) or on the server via `sudo skoci approve <request_id>`.
5. **Zero SSH & SPKI Pinning:** Do not use SSH for normal operations. Connect using `skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json` which persists to `~/.config/skoci/servers.json` (`0600`) by default (pass `--project-env` to also write `.skoci/env` `0600`; legacy `.env.skoci` and root `skoci.yaml` are migrated automatically).
6. **Production Requires Real Domains:** Production deployments reject magic DNS hostnames (`*.sslip.io`) unless the server operator enabled `--allow-magic-dns-production`. Always verify custom domain DNS records before deploying with `skoci domain check <host> --json`.
7. **Token privacy & Log Redaction:** Never print, log, or store plain-text tokens in logs or git repositories. Logs automatically redact secrets, Skoci tokens, and Bearer authorization headers.
8. **Concurrency:** Deploy, promote, rollback, and delete operations on the same app are mutually exclusive; concurrent attempts return `conflict` error naming the active deployment/deletion ID.
9. **Remote builds:** All Docker container builds run on the remote Skoci server; Docker is not required on the local workstation.
## Common Sequences
```bash
# Connect workspace to server with SPKI certificate pin
skoci connect --server <server-ip> --token <token> --pin sha256:<spki-pin> --json

# Verify custom domain DNS record before deploying
skoci domain check <host> --json
# Initialize project (infers app name from manifest, non-root Dockerfile, port 8080 for static sites, strips .skoci/)
skoci init --json

# Offline configuration validation and upload manifest inspection (zero network, exit 0 valid, 1 invalid)
skoci validate --verbose --json

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
