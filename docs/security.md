# Security Model & Defense Architecture

The threat model for an agent-native deployment platform differs fundamentally from traditional PaaS: **commands originate from AI agents susceptible to Prompt Injection attacks.**

Every layer of Skoci treats agent input as untrusted data and enforces defense-in-depth security boundaries.

---

## The 7 Pillars of Defense

```
[AI Agent / External Prompt]
          │
          ▼
┌───────────────────────────────────────────────┐
│ 1. Zero Shell String (Argv Only)              │ ◄── tokio::process::Command without sh -c
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 2. Scoped Token & SHA-256 Hash                │ ◄── DB hash lookup; app-level isolation
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 3. Zero-SSH Key Policy                        │ ◄── No root SSH keys stored on workstations
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 4. Node Secret Encryption (ChaCha20-Poly1305) │ ◄── Secrets masked & injected via 0600 file
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 5. Container Network Isolation & Local Bind   │ ◄── API bound to 127.0.0.1; per-app network
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 6. Human Gate Approval Requirement           │ ◄── Built-in policy: prod actions need review
└───────────────────────┬───────────────────────┘
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ 7. Append-Only Audit Log                      │ ◄── Actor attribution; tokens never stored
└───────────────────────────────────────────────┘
```

---

### Pillar 1: Shell Injection Immunity (Zero Shell Concatenation)
* **Vulnerability in Traditional Tools:** Many PaaS platforms construct commands by interpolating shell strings:
  `sh -c "docker run -e KEY=$input ..."`
  If input contains characters such as `;`, `&`, `|`, or `$()`, an attacker gains immediate shell access on the host.
* **Skoci's Solution:** Skoci **NEVER** invokes a shell with concatenated strings derived from user or agent input. Skoci invokes Docker binary processes directly using structured argument vectors (`tokio::process::Command::new("docker").args(...)`). Special characters are treated strictly as literal arguments. The only shell commands allowed are fixed, static literals that reference the container's own internal environment variables.

---

### Pillar 2: Isolated Scoped Tokens & SHA-256 Hash Verification
* Agent tokens formatted as `sk_agent_<app>_<48 hex>` are stored in the SQLite database exclusively as **SHA-256 hashes**.
* Application scope is determined by database query results from the hash lookup, never by parsing token strings.
* A token for app `toko` is rejected immediately if it attempts to access app `blog` (`scope_violation`).
* **Instant Revocation:** Administrators can revoke tokens at any time:
  ```bash
  skoci token revoke --app toko --name laptop
  ```
* **Brute-Force Protection:** Failed authentication attempts are limited to 10 per minute per peer IP before being blocked with `rate_limited`.

---

### Pillar 3: Zero-SSH Policy (No Root SSH Keys)
* Tools like Kamal require developers and CI runners to store root SSH private keys. If a developer workstation or AI agent sandbox is compromised, the attacker takes complete control of the host.
* **Skoci eliminates SSH for day-to-day operations:** Agents communicate exclusively over HTTPS REST API and MCP authenticated by scoped application tokens.

---

### Pillar 4: Secret Encryption at Rest & Secure Injection
* Secrets are encrypted at rest using **ChaCha20-Poly1305** with the node key `/var/lib/skoci/node.key` (a 32-byte random key generated on first startup, permissions `0600`, readable only by the `skoci` user).
* AI agents can write secrets (`skoci secret set KEY VAL`), but **can never read their values back** (`skoci secret list` returns only key names). Secret values containing newline characters (`\n`) are rejected.
* **Leak-Free Injection:** When containers start, secrets are written to a temporary environment file with `0600` permissions and passed via `docker run --env-file`. The temporary file is deleted from disk immediately after the container starts.
* **Log Redaction:** Any secret value of length >= 4 characters is automatically scrubbed from container logs and replaced with `[REDACTED]`.

---

### Pillar 5: Local Bind Isolation & Container Networks
* **Local Bind `127.0.0.1:8787`:** The Skoci Control API binds strictly to `127.0.0.1:8787` and is never exposed directly to public interfaces. External traffic routes through the TLS reverse proxy or an SSH tunnel.
* **Dashboard Session Cookie:** The web dashboard uses a `skoci_session` cookie (HttpOnly, SameSite=Strict, Secure over HTTPS) with a 12-hour expiry, and enforces the CSRF header `X-Skoci-Dashboard: 1` on all mutating requests.
* **Constant-Time Verification:** Master tokens are validated using constant-time string comparisons to prevent timing attacks.
* **Container Network Isolation:** Each application and its accessories run inside a dedicated Docker bridge network (`skoci-<app>`). Containers for app `toko` cannot access or interfere with containers for app `blog`.

---

### Pillar 6: Systemd Hardening & Docker Group Caveat
The Skoci background service runs under a hardened systemd unit:
* `User=skoci`, `Group=skoci`
* `NoNewPrivileges=true`
* `ProtectSystem=strict`
* `ProtectHome=true`
* `PrivateTmp=true`
* `ReadWritePaths=/var/lib/skoci`
* `AmbientCapabilities=CAP_NET_BIND_SERVICE` (enabling ports 80 and 443 binding without root).

> **Security Notice (Docker Group Caveat):**  
> Any user or service belonging to the `docker` group has access to `/var/run/docker.sock`. On Linux, access to the Docker socket is functionally equivalent to root (`docker group ≈ root`). Skoci isolates command execution and enforces strict argument validation to minimize exposure.

---

### Pillar 7: Upload Limits & Append-Only Audit Logging
* **Upload Limits:** Archive uploads are capped at 200 MB (`--max-upload-mb`) to protect against resource exhaustion attacks.
* **Audit Logging:** All actions are recorded in an append-only SQLite `audit_logs` table with actor attribution (`admin` or `agent:<app>/<name>#<id12>`). Plain-text tokens are never stored in audit logs.
