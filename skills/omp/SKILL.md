---
name: skoci
description: Deploy, monitor, and manage containerized services on VPS using the lightweight Skoci platform.
---

# Skoci Harness Skill for OMP

Use this skill when deploying or managing applications via Skoci.

## Rules
1. Always run commands with `--json` for structured, machine-parsable outputs.
2. In case of error, parse `{ "code": "...", "message": "...", "fix": "..." }` and execute the suggested `fix`.
3. Never attempt to approve production deployments directly. When `promote` returns `pending_approval`, inform the human user of the `request_id`.
4. Store secrets via `skoci secret set <KEY> <VALUE>`. Never commit secrets to `skoci.yaml`.

## Common Sequences
- Initialize project: `skoci init --json`
- Deploy to preview: `skoci deploy --env preview --json`
- Check health: `skoci status --json`
- View errors: `skoci logs --json`
- Promote to prod: `skoci promote "Ready for v1 release" --json`
