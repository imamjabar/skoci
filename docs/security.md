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
* **Credential Entry via Stdin:** To prevent credentials from leaking in shell history or process listings (`ps`, `/proc/<pid>/cmdline`), `skoci connect` accepts tokens via standard input (or `SKOCI_TOKEN`), and `skoci secret set KEY` accepts secret values via standard input when the value parameter is omitted.
* **Leak-Free Container Injection:** When containers start, secrets are written to an ephemeral environment file with `0600` permissions and passed via `docker run --env-file`. The temporary file is securely unlinked immediately after container startup.
* **Environment Sanitization & Server Isolation:** The Skoci server removes `SKOCI_MASTER_TOKEN` from its own process environment immediately after initialization. Child processes (e.g. `docker`, `git`) run in cleaned environments. On the developer/agent workstation, `.env.skoci` is restricted to an explicit allowlist (`SKOCI_SERVER_URL` and `SKOCI_TOKEN`); all other variables are ignored, and spawned child processes (such as `git rev-parse`) clear inherited variables to prevent `LD_PRELOAD` or `DYLD_*` code execution attacks.
* **Packaging Deny List:** Project uploads (`pack_context`) enforce a non-overridable sensitive file deny list: `.env`, `.env.*` (except `.env.example`, `.env.sample`, `.env.template`), `*.pem`, `*.key`, `id_rsa*`, `id_ed25519*`, `id_ecdsa*`, `.npmrc`, `.pypirc`, `.netrc`, `.git-credentials`, and directories `.aws`, `.ssh`, `.docker`, `.gnupg`. Packed archive temp files are created with private `0600` permissions.
* **Log Redaction:** Any secret value of length >= 4 characters is automatically scrubbed from container logs and replaced with `[REDACTED]`.

### Pillar 5: Local Bind Isolation & Container Networks
* **Local Bind `127.0.0.1:8787`:** The Skoci Control API binds strictly to `127.0.0.1:8787` and is never exposed directly to public interfaces. External traffic routes through the TLS reverse proxy or an SSH tunnel.
* **HTTPS SNI Binding:** On the HTTPS listener, the HTTP/2 `:authority` or HTTP/1 `Host` header must strictly match the TLS SNI (Server Name Indication) negotiated during the handshake. If they differ, or if an empty/IP-literal host is sent over HTTPS, the proxy returns `421 Misdirected Request`. This prevents domain fronting and virtual host routing confusion. Default app fallback applies solely to plain-HTTP listeners for IP-literal or empty hosts.
* **Host Ownership & Collision Protection:** An agent may route only its own auto hosts (`<app>.<base>`, `<app>-preview.<base>`) and custom domains that are not under `<base>`, not equal to the control/API domain, and not owned by another app. Host ownership is established on first use (any route entry or pending/active deployment); subsequent attempts by other apps or environments to claim an existing host are rejected with `conflict`. Admin deployments follow the exact same ownership rules.
* **Dashboard Session Cookie:** The web dashboard uses a `skoci_session` cookie (HttpOnly, SameSite=Strict, Secure over HTTPS) with a 12-hour expiry, and enforces the CSRF header `X-Skoci-Dashboard: 1` on all mutating requests.
* **Constant-Time Verification:** Master tokens are validated using constant-time string comparisons to prevent timing attacks.
* **Container Network Isolation:** Each application and its accessories run inside a dedicated Docker bridge network (`skoci-<app>`). Containers for app `toko` cannot access or interfere with containers for app `blog`.

### Pillar 6: Systemd Hardening & Docker Group Caveat
The Skoci background service runs under a strictly confined systemd unit:
* `User=skoci`, `Group=skoci`, `SupplementaryGroups=docker`
* `UMask=0077` (all created files default to restrictive owner-only permissions)
* `LimitNOFILE=65536` (high file descriptor ceiling for concurrent proxy connections)
* `NoNewPrivileges=true`
* `ProtectSystem=strict`
* `ProtectHome=true`
* `PrivateTmp=true`
* `PrivateDevices=true` (isolated pseudo-device tree, preventing direct hardware access)
* `ProtectKernelTunables=true` (`/proc/sys`, `/sys` mounted read-only)
* `ProtectKernelModules=true` (disallows kernel module loading)
* `ProtectControlGroups=true` (cgroup hierarchy mounted read-only)
* `RestrictSUIDSGID=true` (prevents acquiring SUID/SGID privileges)
* `LockPersonality=true` (locks execution domain)
* `RestrictNamespaces=true` (disallows unprivileged namespace creation)
* `ReadWritePaths=/var/lib/skoci`
* `AmbientCapabilities=CAP_NET_BIND_SERVICE` and `CapabilityBoundingSet=CAP_NET_BIND_SERVICE` (enabling low-port binding 80/443 without root)
* Directory permissions: `/var/lib/skoci` mode `0750`, `/etc/skoci` mode `0750`, configuration and token files mode `0640`.

### Pillar 7: Upload Limits & Append-Only Audit Logging
* **Upload Limits:** Archive uploads are capped at 200 MB (`--max-upload-mb`) to protect against resource exhaustion attacks.
* **Audit Logging:** All actions are recorded in an append-only SQLite `audit_logs` table with actor attribution (`admin` or `agent:<app>/<name>#<id12>`). Plain-text tokens are never stored in audit logs.
* **Approval Pinning to Image ID:** Production releases, database restorations, and destructive operations enforce human sign-off. Approval requests pin the exact Docker image ID/digest produced during the build, rather than mutable tags or deploy names. This eliminates time-of-check to time-of-use (TOCTOU) replacement attacks where an image could be tampered with between approval creation and promotion.

---

## Build Provenance & Supply Chain Security
Release binaries are compiled in isolated GitHub Actions runners with minimal privileges (`permissions: {}` top-level, `contents: read` during compilation, and zero persisted checkout credentials). Releases include cryptographic build provenance attestations generated via GitHub Actions:
```bash
# Verify release binary authenticity and provenance:
gh attestation verify skoci-linux-x86_64 --repo imamjabar/skoci
```

---

## Residual Risks

While Skoci implements strict defense-in-depth boundaries, operators must be aware of the following residual risks inherent in single-host container platforms:

1. **`docker group ≈ root`:** Any user or service belonging to the Linux `docker` group possesses access to `/var/run/docker.sock`. On Linux, access to the Docker socket is functionally equivalent to host root access. Skoci hardens its service process to mitigate injection, but socket access remains root-equivalent.
2. **Unrestricted Container Outbound Network:** AI coding agents deploy and execute arbitrary code within their designated container sandbox. Container outbound network connections are unrestricted by default (containers can establish outbound connections to internet destinations).
3. **First-Come Custom Domain Claiming:** Custom domains operate on a first-come, first-served basis. Any agent can claim an unclaimed custom domain for its application as long as no other application on the instance currently uses it.
4. **Same-Origin Release Checksums:** The `.sha256` checksum files are hosted within the same GitHub Release assets as the binaries themselves. To verify release integrity against repository tampering, operators should use GitHub artifact attestations (`gh attestation verify skoci-linux-x86_64 --repo imamjabar/skoci`).
5. **Single-Server Docker Isolation:** Skoci is designed for single-node deployments and relies on standard Docker container boundaries (Linux cgroups and namespaces). It does not provide hypervisor or microVM-level isolation (e.g. Firecracker or gVisor).
6. **ACME & sslip.io Rate Limits:** Automatic TLS provisioning using `sslip.io` subdomains shares global Let's Encrypt rate limits across all internet users of the public `sslip.io` service. For mission-critical production workloads, operators should configure custom domains with dedicated DNS.
