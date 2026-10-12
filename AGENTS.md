# Skoci Agent & Architecture Rules

## Core Principles

1. **Lightweight (<20MB idle RAM):**
   - Single Rust binary containing control plane, node, proxy, and dashboard.
   - Ephemeral tasks (builds, backups, migrations) run on demand and exit immediately.
   - Do not leak background threads, caches, or unmanaged buffers.

2. **Single Source of Truth (`skoci-api`):**
   - All request/response contracts, policy schemas, evidence formats, and errors live in `skoci-api`.
   - Error format MUST always serialize to `{ "code": "...", "message": "...", "fix": "..." }`.
   - Never define ad-hoc types in CLI or MCP that diverge from `skoci-api`.

3. **Safe by Design:**
   - NEVER interpolate agent-provided strings into shell commands.
   - All Docker and system operations must use structured API calls.
   - Secrets are encrypted at rest using node keys and NEVER returned to agent context.
   - Destructive operations (`delete`, `db_restore`, `upgrade`, production `deploy`) require human approval.

4. **Structured Agent Output:**
   - Every CLI command supports `--json` and emits consistent exit codes (0 for success, non-zero on failure).
   - Every response provides both structured machine data and a 1-sentence plain text `summary`.
   - MCP tools mirror CLI commands, with human-only actions (`approve`, `upgrade`) strictly excluded from MCP schemas.

5. **Code Style & Organization:**
   - Async runtime: Tokio.
   - Error handling: `thiserror` in libraries, structured `SkociError` in `skoci-api`.
   - Keep crate boundaries clean and dependencies minimal.

6. **Documentation Ships With the Release:**
   - `README.md` and `README.id.md` MUST be updated in every release; the Indonesian file mirrors the English one heading for heading.
   - A new or changed CLI flag or subcommand is not done until it appears in the READMEs, `docs/`, or `skills/llms.txt`.
   - Version samples in `docs/` track the workspace version in `Cargo.toml`.
   - `scripts/check_docs.py` enforces all of the above on every push; the release workflow additionally runs it with `--since <previous tag>` and refuses to build a tag whose READMEs are unchanged.
