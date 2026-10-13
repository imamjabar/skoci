# Deploy Databases & Manage Backups on Skoci

Skoci supports provisioning managed database accessory services that run in isolated containers with persistent storage volumes and dedicated internal networks.

---

## 1. Supported Database Engines

* **PostgreSQL:** Versions `15`, `16`, `17` (Official image: `postgres:<version>`)
* **MySQL:** Versions `8.0`, `8.4` (Official image: `mysql:<version>`)
* **Valkey / Redis:** Versions `7.2`, `8.0` (Official image: `valkey/valkey:<version>`)

---

## 2. Declaring Databases in `.skoci/skoci.yaml`

Databases are declared under the `services` section of `.skoci/skoci.yaml` in your application repository:

```yaml
version: 1
app: toko
run:
  port: 3000
  health: /healthz
services:
  db:
    engine: postgres
    version: "16"
    memory: 512m          # Database container memory limit
    backup:
      schedule: "0 2 * * *"  # Cron schedule in UTC (daily at 02:00 AM)
      keep: 7                # Keep 7 most recent backup snapshots
```

### Network & Security Properties:
1. **Isolated Internal Network:** Database containers join an isolated Docker bridge network `skoci-<app>` and do not publish ports to the public host interface.
2. **Persistent Volumes:** Data is preserved on a named Docker volume `skoci-<app>-<service>-data` (e.g. `skoci-toko-db-data`). Data persists across app redeploys.
3. **Automatic Credential Injection:**
   - Skoci generates a random 32-character alphanumeric password and saves it as an encrypted secret `SKOCI_<SERVICE_UPPER>_PASSWORD`.
   - Connection URLs are automatically injected into the application container:
     - A service named `db` is injected as `DATABASE_URL`.
     - A service with another name (e.g. `cache`) is injected as `<NAME_UPPER>_URL` (e.g. `CACHE_URL`).
   - Connection URL formats:
     - **Postgres:** `postgresql://skoci:<password>@db:5432/<app_name>`
     - **MySQL:** `mysql://skoci:<password>@<service>:3306/<app_name>`
     - **Valkey:** `redis://:<password>@<service>:6379`

---

## 3. Creating Immediate Database Backups (`skoci db backup`)

In addition to scheduled cron backups, you or an AI agent can trigger an on-demand snapshot before running data migrations:

```bash
skoci db backup db
```

Structured output:
```json
{
  "success": true,
  "data": {
    "backup_id": "db-20261011024500",
    "app": "toko",
    "service": "db",
    "size_bytes": 10485760,
    "location": "/var/lib/skoci/backups/toko/db/db-20261011024500.dump",
    "created_at": "2026-10-11T02:45:00Z"
  },
  "summary": "Database backup 'db' created successfully"
}
```

Backup file formats:
* PostgreSQL: `.dump` (via `pg_dump -Fc`)
* MySQL: `.sql` (via `mysqldump`)
* Valkey: `.rdb` (via `SAVE` snapshot)

List existing backups at any time:
```bash
skoci db list
```

---

## 4. Restoring Databases (`skoci db restore`)

Database restoration is a destructive action that can overwrite active runtime data.

By default on Skoci servers, **database restoration by an agent always requires human operator approval**:

### Restoration Workflow:
1. The AI agent or developer requests restoration:
   ```bash
   skoci db restore db-20261011024500 db
   ```
2. Skoci records the restore request and returns status `pending_approval` with a `request_id`.
3. A human operator validates and approves the restore via the **Web Dashboard** at `https://skoci.<base-domain>` or via the server CLI:
   ```bash
   sudo skoci approve req_restore_9876
   ```
4. Skoci atomically restores the data archive into the database container.
