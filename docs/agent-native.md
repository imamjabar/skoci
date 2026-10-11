# Agen Coding Sebagai Operator (Agent-Native Architecture)

Alat deploy tradisional (seperti Coolify dan Dokploy) dirancang untuk diklik manusia melalui ratusan form antarmuka web, sementara integrasi agen AI hanyalah tempelan tambahan lewat plugin eksternal.

**Skoci dirancang sejak hari pertama dengan prinsip: Agen AI yang men-deploy dan memantau, Manusia yang memvalidasi dan menyetujui.**

---

## 1. Integrasi MCP (Model Context Protocol)

Skoci menyediakan server MCP bawaan yang berkomunikasi melalui standard `stdio` JSON-RPC 2.0.

### A. Konfigurasi Claude Code (`.mcp.json` atau `~/.claude.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "/usr/local/bin/skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "http://IP_SERVER_ANDA:8787",
        "SKOCI_TOKEN": "sk_agent_toko_xxxx"
      }
    }
  }
}
```

### B. Konfigurasi OMP (`~/.omp/agent/config.yml` atau `.omp/mcp.json`)
```json
{
  "mcpServers": {
    "skoci": {
      "command": "skoci",
      "args": ["mcp"],
      "env": {
        "SKOCI_SERVER_URL": "http://IP_SERVER_ANDA:8787",
        "SKOCI_TOKEN": "sk_agent_toko_xxxx"
      }
    }
  }
}
```

### C. Konfigurasi Pi, Codex, OpenClaw, atau DSH
Gunakan binary `skoci mcp` dengan environment variable `SKOCI_SERVER_URL` dan `SKOCI_TOKEN`.

---

## 2. Daftar Tool MCP yang Tersedia

Semua tool dihasilkan dari satu sumber kebenaran API (`skoci-api`), menjamin paritas 100% dengan CLI:

| Nama Tool | Fungsi | Parameter |
| :--- | :--- | :--- |
| `skoci_init` | Mendeteksi framework & membuat `skoci.yaml` | `app`, `path` |
| `skoci_deploy` | Build, run, health check, & cutover | `app`, `env` (`preview`/`production`), `force` |
| `skoci_status` | Memeriksa kondisi container & port | `app` |
| `skoci_logs` | Mengambil log terbaru dengan sanitasi secret | `app`, `tail` |
| `skoci_secret_set` | Menyimpan secret terenkripsi | `app`, `key`, `value` |
| `skoci_secret_list` | Melihat nama-nama secret terdaftar | `app` |
| `skoci_promote` | Mengajukan izin promosi ke production | `app`, `reason` |
| `skoci_rollback` | Kembali ke versi sehat sebelumnya | `app`, `target_version` |
| `skoci_doctor` | Memeriksa RAM idle (<20MB) & status Docker | *(tanpa parameter)* |
| `skoci_db_backup` | Memicu snapshot backup database | `app`, `service` |
| `skoci_db_restore` | Meminta pemulihan database | `app`, `service`, `backup_id` |

---

## 3. Batas Keamanan Tool (Strict Human Exclusion)

Perintah berikut **secara sengaja DIKECUALIKAN dari definisi tool MCP**:
1. `skoci_approve`: Agen **TIDAK BISA** menyetujui permintaannya sendiri. Persetujuan promosi production mutlak berada di tangan operator manusia via Dashboard atau token master.
2. `skoci_upgrade`: Agen **TIDAK BISA** mengganti binary inti platform.

Jika agen mencoba memanggil fungsi ini lewat API, lapisan otentikasi akan menolak dengan error `unauthorized: Hanya manusia (admin) yang dapat menyetujui approval`.

---

## 4. Penanganan Error `{code, message, fix}`

Setiap kali terjadi galat, Skoci tidak mengembalikan stack trace mentah atau teks kabur. Skoci mengembalikan format terstruktur yang dapat dipahami langsung oleh model LLM:

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

Agen membaca kolom `fix` dan dapat langsung mengeksekusi instruksi perbaikan tanpa menebak penyebab kegagalan.

---

## 5. Panduan Agen Bawaan (`skoci agent-guide`)

Skoci menyediakan perintah bawaan untuk mencetak ringkasan panduan kerja agen dalam format JSON atau Markdown:

```bash
skoci agent-guide --json
```

File skill untuk berbagai harness coding juga tersedia langsung di repositori:
* `skills/omp/SKILL.md`
* `skills/claude-code/SKILL.md`
* `skills/llms.txt`
