---
name: skoci
description: Operator skill for deploying and managing services on Skoci platform.
---

# Skoci Skill for Claude Code

## Commands
```bash
skoci init --json
skoci deploy --env preview --json
skoci status --json
skoci logs --json
skoci secret set KEY VALUE --json
skoci promote "Launch ready" --json
```

## Error Protocol
When command fails:
- Check `code` (e.g. `secret_missing`)
- Review `message`
- Run the suggested `fix` command
