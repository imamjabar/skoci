use crate::config::is_valid_app_name;
use skoci_api::models::{BackupPolicy, DatabaseEngine, ServiceConfig, MAX_APP_NAME_LEN};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    NodeServer,
    StaticBuild,
    Static,
    Python,
    Rust,
    Go,
    Php,
    Docker,
}

impl Framework {
    pub fn name(&self) -> &'static str {
        match self {
            Framework::NodeServer => "Node.js server",
            Framework::StaticBuild => "Static build (Node + nginx)",
            Framework::Static => "Static (nginx)",
            Framework::Python => "Python",
            Framework::Rust => "Rust",
            Framework::Go => "Go",
            Framework::Php => "PHP",
            Framework::Docker => "Docker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub framework: Framework,
    pub port: u16,
    pub health: String,
    pub services: BTreeMap<String, ServiceConfig>,
    pub notes: Vec<String>,
}

pub struct ProjectDetector;

impl ProjectDetector {
    pub fn suggest_app_name(dir: &Path) -> Option<String> {
        let pkg_path = dir.join("package.json");
        if pkg_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&pkg_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(raw) = json.get("name").and_then(|v| v.as_str()) {
                        let stripped = if raw.starts_with('@') {
                            if let Some(slash_idx) = raw.find('/') {
                                &raw[slash_idx + 1..]
                            } else {
                                raw
                            }
                        } else {
                            raw
                        };
                        if let Some(name) = normalize_app_name(stripped) {
                            return Some(name);
                        }
                    }
                }
            }
        }

        let cargo_path = dir.join("Cargo.toml");
        if cargo_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&cargo_path) {
                if let Some(raw) = extract_toml_name(&content, "package") {
                    if let Some(name) = normalize_app_name(&raw) {
                        return Some(name);
                    }
                }
            }
        }

        let pyproject_path = dir.join("pyproject.toml");
        if pyproject_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&pyproject_path) {
                if let Some(raw) = extract_toml_name(&content, "project") {
                    if let Some(name) = normalize_app_name(&raw) {
                        return Some(name);
                    }
                }
                if let Some(raw) = extract_toml_name(&content, "tool.poetry") {
                    if let Some(name) = normalize_app_name(&raw) {
                        return Some(name);
                    }
                }
            }
        }

        None
    }

    pub fn detect(dir: &Path) -> Detection {
        let mut notes = Vec::new();
        let services = scan_for_services(dir);

        // 1. Existing Dockerfile
        let dockerfile_path = if dir.join("Dockerfile").is_file() {
            Some(dir.join("Dockerfile"))
        } else if dir.join("dockerfile").is_file() {
            Some(dir.join("dockerfile"))
        } else {
            None
        };

        if let Some(path) = dockerfile_path {
            let mut port = 8080;
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed
                        .strip_prefix("EXPOSE ")
                        .or_else(|| trimmed.strip_prefix("expose "))
                    {
                        let port_str = rest.trim().split(['/', ' ']).next().unwrap_or("");
                        if let Ok(p) = port_str.parse::<u16>() {
                            if p > 0 {
                                port = p;
                                break;
                            }
                        }
                    }
                }
            }
            notes.push("Using existing Dockerfile found in the project.".to_string());
            notes.push(
                "Ensure the container provides a '/healthz' endpoint or change 'run.health' in .skoci/skoci.yaml."
                    .to_string(),
            );
            return Detection {
                framework: Framework::Docker,
                port,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        // 2. package.json
        let pkg_path = dir.join("package.json");
        if pkg_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&pkg_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    let scripts = json.get("scripts").and_then(|s| s.as_object());
                    let has_start = scripts.map(|s| s.contains_key("start")).unwrap_or(false);
                    let has_build = scripts.map(|s| s.contains_key("build")).unwrap_or(false);

                    let has_lockfile = dir.join("package-lock.json").is_file()
                        || dir.join("npm-shrinkwrap.json").is_file();

                    if has_start {
                        notes.push("Detected Node.js with 'start' script.".to_string());
                        if has_lockfile {
                            notes.push("Using 'npm ci' because package-lock.json was found.".to_string());
                        } else {
                            notes.push("Using 'npm install'.".to_string());
                        }
                        notes.push(
                            "Health check set to '/' so initial deployment succeeds immediately. Change to '/healthz' once a dedicated endpoint exists."
                                .to_string(),
                        );
                        return Detection {
                            framework: Framework::NodeServer,
                            port: 3000,
                            health: "/".to_string(),
                            services,
                            notes,
                        };
                    } else if has_build {
                        let candidates = ["dist", "build", "out", "public"];
                        let mut out_dir = "dist";
                        for cand in candidates {
                            if dir.join(cand).is_dir() {
                                out_dir = cand;
                                break;
                            }
                        }
                        notes.push(
                            "Detected static build (Node.js build with nginx web server)."
                                .to_string(),
                        );
                        notes.push(format!("Build output directory: '{out_dir}'"));
                        if has_lockfile {
                            notes.push("Using 'npm ci' because package-lock.json was found.".to_string());
                        } else {
                            notes.push("Using 'npm install'.".to_string());
                        }
                        return Detection {
                            framework: Framework::StaticBuild,
                            port: 80,
                            health: "/healthz".to_string(),
                            services,
                            notes,
                        };
                    }
                }
            }
        }

        // 3. requirements.txt or pyproject.toml
        let has_req = dir.join("requirements.txt").is_file();
        let has_pyproject = dir.join("pyproject.toml").is_file();
        if has_req || has_pyproject {
            let mut all_text = String::new();
            if let Ok(c) = std::fs::read_to_string(dir.join("requirements.txt")) {
                all_text.push_str(&c);
                all_text.push(' ');
            }
            if let Ok(c) = std::fs::read_to_string(dir.join("pyproject.toml")) {
                all_text.push_str(&c);
            }
            let lower = all_text.to_ascii_lowercase();

            notes.push("Detected Python project.".to_string());
            if lower.contains("fastapi") {
                notes.push("Server: uvicorn main:app".to_string());
            } else if lower.contains("flask") || lower.contains("django") {
                notes.push("Server: gunicorn main:app".to_string());
            } else {
                notes.push("Server: python main.py".to_string());
            }
            notes.push(
                "Health check set to '/' so initial deployment succeeds immediately. Change to '/healthz' once a dedicated endpoint exists."
                    .to_string(),
            );

            return Detection {
                framework: Framework::Python,
                port: 8000,
                health: "/".to_string(),
                services,
                notes,
            };
        }

        // 4. Cargo.toml
        let cargo_path = dir.join("Cargo.toml");
        if cargo_path.is_file() {
            let bin_name = extract_rust_bin_name(&cargo_path);
            notes.push("Detected Rust project.".to_string());
            notes.push(format!("Rust binary: '{bin_name}'"));
            notes.push(
                "Ensure the application provides a '/healthz' endpoint or change 'run.health' in .skoci/skoci.yaml."
                    .to_string(),
            );
            return Detection {
                framework: Framework::Rust,
                port: 8080,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        // 5. go.mod
        if dir.join("go.mod").is_file() {
            notes.push("Detected Go project.".to_string());
            notes.push(
                "Ensure the application provides a '/healthz' endpoint or change 'run.health' in .skoci/skoci.yaml."
                    .to_string(),
            );
            return Detection {
                framework: Framework::Go,
                port: 8080,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        // 6. composer.json
        if dir.join("composer.json").is_file() {
            notes.push("Detected PHP project.".to_string());
            if dir.join("public").is_dir() {
                notes.push("Apache document root: public/".to_string());
            }
            notes.push(
                "Ensure the application provides a '/healthz' endpoint or change 'run.health' in .skoci/skoci.yaml."
                    .to_string(),
            );
            return Detection {
                framework: Framework::Php,
                port: 80,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        // 7. index.html at root
        if dir.join("index.html").is_file() {
            notes.push("Detected static website (HTML/CSS/JS) with nginx web server.".to_string());
            return Detection {
                framework: Framework::Static,
                port: 80,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        // 8. Fallback: static html depth <= 2 else Docker
        if has_html_within_depth_2(dir) {
            notes.push(
                "Detected HTML files in project directory; served with nginx web server."
                    .to_string(),
            );
            return Detection {
                framework: Framework::Static,
                port: 80,
                health: "/healthz".to_string(),
                services,
                notes,
            };
        }

        notes.push(
            "Could not detect framework automatically. Create a 'Dockerfile' in the project directory to continue."
                .to_string(),
        );
        Detection {
            framework: Framework::Docker,
            port: 8080,
            health: "/healthz".to_string(),
            services,
            notes,
        }
    }

    pub fn generate_skoci_yaml(app: &str, d: &Detection) -> String {
        let mut out = String::new();
        out.push_str("version: 1\n");
        out.push_str(&format!("app: {app}\n"));
        out.push_str("build:\n");
        let df_path = if Self::dockerfile(d).is_some() {
            skoci_api::models::PROJECT_DOCKERFILE_PATH
        } else {
            "Dockerfile"
        };
        out.push_str(&format!("  dockerfile: {df_path}\n"));
        out.push_str("  context: .\n");
        out.push_str("run:\n");
        out.push_str(&format!("  port: {}\n", d.port));
        out.push_str(&format!("  health: \"{}\"\n", d.health));
        out.push_str("  health_timeout_secs: 60\n");
        out.push_str("  memory: 256m\n");
        out.push_str("# Add your domains here when ready (e.g. - app.yourdomain.com)\n");
        out.push_str("domains: []\n");
        out.push_str("preview_domains: []\n");
        out.push_str("env:\n");
        out.push_str("  public: {}\n");
        out.push_str("  secrets: []\n");

        if !d.services.is_empty() {
            out.push_str("services:\n");
            for (name, svc) in &d.services {
                out.push_str(&format!("  {name}:\n"));
                let engine_str = match svc.engine {
                    DatabaseEngine::Postgres => "postgres",
                    DatabaseEngine::Mysql => "mysql",
                    DatabaseEngine::Valkey => "valkey",
                };
                out.push_str(&format!("    engine: {engine_str}\n"));
                out.push_str(&format!("    version: \"{}\"\n", svc.version));
                if let Some(mem) = &svc.memory {
                    out.push_str(&format!("    memory: {mem}\n"));
                }
                if let Some(backup) = &svc.backup {
                    out.push_str("    backup:\n");
                    out.push_str(&format!("      schedule: \"{}\"\n", backup.schedule));
                    out.push_str(&format!("      keep: {}\n", backup.keep));
                }
            }
        }

        out
    }

    pub fn dockerfile(d: &Detection) -> Option<String> {
        match d.framework {
            Framework::Docker => None,
            Framework::NodeServer => {
                let install_cmd = if d
                    .notes
                    .iter()
                    .any(|n| n.contains("npm ci") || n.contains("package-lock.json"))
                {
                    "npm ci"
                } else {
                    "npm install"
                };
                Some(format!(
r#"FROM node:22-alpine
WORKDIR /app
COPY package*.json ./
RUN {install_cmd}
COPY . .
RUN rm -rf /app/.skoci
RUN npm run build --if-present
RUN chown -R node:node /app
USER node
ENV PORT=3000 HOST=0.0.0.0
EXPOSE 3000
CMD ["npm", "start"]
"#
                ))
            }
            Framework::StaticBuild => {
                let mut out_dir = "dist";
                for note in &d.notes {
                    if let Some(pos) = note.find("Build output directory: '") {
                        let rest = &note[pos + "Build output directory: '".len()..];
                        if let Some(end) = rest.find('\'') {
                            out_dir = &rest[..end];
                            break;
                        }
                    }
                }
                let install_cmd = if d
                    .notes
                    .iter()
                    .any(|n| n.contains("npm ci") || n.contains("package-lock.json"))
                {
                    "npm ci"
                } else {
                    "npm install"
                };
                Some(format!(
r#"FROM node:22-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN {install_cmd}
COPY . .
RUN npm run build

FROM nginx:1.27-alpine
RUN printf 'server {{\n    listen 80;\n    server_name localhost;\n    root /usr/share/nginx/html;\n    index index.html;\n    location = /healthz {{\n        access_log off;\n        return 200 '\''ok'\'';\n    }}\n    location ~ /\\. {{\n        deny all;\n        return 404;\n    }}\n    location / {{\n        try_files $uri $uri.html $uri/ /index.html =404;\n    }}\n}}\n' > /etc/nginx/conf.d/default.conf
COPY --from=builder /app/{out_dir} /usr/share/nginx/html
RUN chmod -R a+rX /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
"#
                ))
            }
            Framework::Static => Some(
r#"FROM nginx:1.27-alpine
RUN printf 'server {\n    listen 80;\n    server_name localhost;\n    root /usr/share/nginx/html;\n    index index.html;\n    location = /healthz {\n        access_log off;\n        return 200 '\''ok'\'';\n    }\n    location ~ /\\. {\n        deny all;\n        return 404;\n    }\n    location / {\n        try_files $uri $uri.html $uri/ =404;\n    }\n}\n' > /etc/nginx/conf.d/default.conf
COPY . /usr/share/nginx/html
RUN rm -rf /usr/share/nginx/html/.skoci
RUN chmod -R a+rX /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
"#.to_string(),
            ),
            Framework::Python => {
                let cmd = if d.notes.iter().any(|n| n.contains("uvicorn")) {
                    r#"CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000"]"#
                } else if d.notes.iter().any(|n| n.contains("gunicorn")) {
                    r#"CMD ["gunicorn", "-b", "0.0.0.0:8000", "main:app"]"#
                } else {
                    r#"CMD ["python", "main.py"]"#
                };
                Some(format!(
r#"FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt* pyproject.toml* ./
RUN if [ -f requirements.txt ]; then pip install --no-cache-dir -r requirements.txt; elif [ -f pyproject.toml ]; then pip install --no-cache-dir .; fi
COPY . .
RUN rm -rf /app/.skoci
RUN useradd -m -u 1000 appuser && chown -R appuser:appuser /app
USER appuser
EXPOSE 8000
{cmd}
"#
                ))
            }
            Framework::Rust => {
                let mut bin_name = "app";
                for note in &d.notes {
                    if let Some(pos) = note.find("Rust binary: '") {
                        let rest = &note[pos + "Rust binary: '".len()..];
                        if let Some(end) = rest.find('\'') {
                            bin_name = &rest[..end];
                            break;
                        }
                    }
                }
                Some(format!(
r#"FROM rust:1.80-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:3.20 AS runner
WORKDIR /app
RUN adduser -D -u 1000 appuser
COPY --from=builder /app/target/release/{bin_name} /app/{bin_name}
RUN chown -R appuser:appuser /app
USER appuser
EXPOSE 8080
CMD ["/app/{bin_name}"]
"#
                ))
            }
            Framework::Go => Some(
r#"FROM golang:1.23-alpine AS builder
WORKDIR /app
COPY go.mod go.sum* ./
RUN go mod download 2>/dev/null || true
COPY . .
RUN CGO_ENABLED=0 go build -o server .

FROM alpine:3.20 AS runner
WORKDIR /app
RUN adduser -D -u 1000 appuser
COPY --from=builder /app/server /app/server
RUN chown -R appuser:appuser /app
USER appuser
EXPOSE 8080
CMD ["/app/server"]
"#.to_string(),
            ),
            Framework::Php => {
                let has_public = d.notes.iter().any(|n| n.contains("public/"));
                let apache_conf = if has_public {
                    r#"RUN sed -ri -e 's!/var/www/html!/var/www/html/public!g' /etc/apache2/sites-available/*.conf
RUN sed -ri -e 's!/var/www/!/var/www/html/public!g' /etc/apache2/apache2.conf /etc/apache2/conf-available/*.conf
"#
                } else {
                    ""
                };
                Some(format!(
r#"FROM php:8.3-apache
# Everything in the build context is copied below; keep secrets out of it (.dockerignore).
# Serve only the public/ directory (DocumentRoot) so project files stay unreachable over HTTP.
{apache_conf}RUN a2enmod rewrite
COPY . /var/www/html/
RUN rm -rf /var/www/html/.skoci
EXPOSE 80
"#
                ))
            }
        }
    }

    pub fn upload_ignore() -> &'static str {
        ".git\n.env*\n*.pem\n*.key\nid_rsa*\nid_ed25519*\n.npmrc\n.pypirc\n.netrc\n.aws\n.ssh\nnode_modules\ntarget\n__pycache__\n*.log\n.DS_Store\n"
    }

    #[deprecated(note = "use upload_ignore instead")]
    pub fn dockerignore() -> &'static str {
        Self::upload_ignore()
    }
}

fn scan_for_services(dir: &Path) -> BTreeMap<String, ServiceConfig> {
    let mut has_postgres = false;
    let mut has_mysql = false;
    let mut has_redis = false;

    // 1. package.json
    let pkg_path = dir.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            let mut dep_names = Vec::new();
            if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
                dep_names.extend(deps.keys().cloned());
            }
            if let Some(dev_deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
                dep_names.extend(dev_deps.keys().cloned());
            }
            for name in &dep_names {
                let lower = name.to_ascii_lowercase();
                if lower == "pg" || lower == "postgres" {
                    has_postgres = true;
                }
                if lower == "mysql" || lower == "mysql2" {
                    has_mysql = true;
                }
                if lower == "redis" || lower == "ioredis" || lower == "valkey" {
                    has_redis = true;
                }
            }
        }
    }

    // Prisma schema check (either in prisma/schema.prisma or schema.prisma)
    for prisma_path in &[dir.join("prisma").join("schema.prisma"), dir.join("schema.prisma")] {
        if let Ok(content) = std::fs::read_to_string(prisma_path) {
            let lower = content.to_ascii_lowercase();
            if lower.contains("provider") && (lower.contains("postgresql") || lower.contains("postgres")) {
                has_postgres = true;
            }
            if lower.contains("provider") && lower.contains("mysql") {
                has_mysql = true;
            }
        }
    }

    // 2. requirements.txt / pyproject.toml
    for py_file in &[dir.join("requirements.txt"), dir.join("pyproject.toml")] {
        if let Ok(content) = std::fs::read_to_string(py_file) {
            let lower = content.to_ascii_lowercase();
            for word in lower.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
                match word {
                    "psycopg" | "psycopg2" | "asyncpg" | "postgres" => has_postgres = true,
                    "mysql" | "pymysql" | "mysqlclient" => has_mysql = true,
                    "redis" | "valkey" => has_redis = true,
                    _ => {}
                }
            }
        }
    }

    // 3. Cargo.toml
    if let Ok(content) = std::fs::read_to_string(dir.join("Cargo.toml")) {
        let lower = content.to_ascii_lowercase();
        if lower.contains("postgres") || lower.contains("pgx") {
            has_postgres = true;
        }
        if lower.contains("mysql") {
            has_mysql = true;
        }
        if lower.contains("redis") || lower.contains("valkey") {
            has_redis = true;
        }
    }

    // 4. go.mod
    if let Ok(content) = std::fs::read_to_string(dir.join("go.mod")) {
        let lower = content.to_ascii_lowercase();
        if lower.contains("lib/pq") || lower.contains("pgx") || lower.contains("postgres") {
            has_postgres = true;
        }
        if lower.contains("mysql") {
            has_mysql = true;
        }
        if lower.contains("redis") || lower.contains("valkey") {
            has_redis = true;
        }
    }

    // 5. composer.json
    if let Ok(content) = std::fs::read_to_string(dir.join("composer.json")) {
        let lower = content.to_ascii_lowercase();
        if lower.contains("pgsql") || lower.contains("postgres") {
            has_postgres = true;
        }
        if lower.contains("mysql") || lower.contains("pdo_mysql") {
            has_mysql = true;
        }
        if lower.contains("predis") || lower.contains("redis") || lower.contains("valkey") {
            has_redis = true;
        }
    }

    let mut services = BTreeMap::new();
    if has_postgres {
        services.insert(
            "db".to_string(),
            ServiceConfig {
                engine: DatabaseEngine::Postgres,
                version: "16".to_string(),
                memory: None,
                backup: Some(BackupPolicy {
                    schedule: "0 2 * * *".to_string(),
                    keep: 7,
                }),
            },
        );
    } else if has_mysql {
        services.insert(
            "db".to_string(),
            ServiceConfig {
                engine: DatabaseEngine::Mysql,
                version: "8.4".to_string(),
                memory: None,
                backup: Some(BackupPolicy {
                    schedule: "0 2 * * *".to_string(),
                    keep: 7,
                }),
            },
        );
    }

    if has_redis {
        services.insert(
            "cache".to_string(),
            ServiceConfig {
                engine: DatabaseEngine::Valkey,
                version: "8".to_string(),
                memory: None,
                backup: None,
            },
        );
    }

    services
}

fn has_html_within_depth_2(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if path.extension().and_then(|s| s.to_str()) == Some("html") {
                return true;
            }
        } else if path.is_dir() {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();
            if name_str.starts_with('.')
                || name_str == "node_modules"
                || name_str == "target"
                || name_str == "vendor"
                || name_str == "__pycache__"
            {
                continue;
            }
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && sub_path.extension().and_then(|s| s.to_str()) == Some("html")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn extract_rust_bin_name(cargo_path: &Path) -> String {
    if let Ok(content) = std::fs::read_to_string(cargo_path) {
        if let Some(name) = extract_toml_name(&content, "package") {
            if !name.is_empty() {
                return name;
            }
        }
    }
    "app".to_string()
}

/// Best-effort normalisation of an arbitrary string into the app-name alphabet.
/// The result is not guaranteed to be a valid app name; callers that need a
/// guarantee use [`normalize_app_name`], callers that need to report *why* a
/// candidate is unusable keep the lossy form.
pub fn normalize_app_name_lossy(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut collapsed = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for c in lower.chars() {
        let mapped = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            c
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash {
                collapsed.push('-');
                prev_dash = true;
            }
        } else {
            collapsed.push(mapped);
            prev_dash = false;
        }
    }
    let trimmed = collapsed.trim_matches('-');
    let truncated = if trimmed.len() > MAX_APP_NAME_LEN {
        &trimmed[..MAX_APP_NAME_LEN]
    } else {
        trimmed
    };
    truncated.trim_end_matches('-').to_string()
}

pub fn normalize_app_name(raw: &str) -> Option<String> {
    let candidate = normalize_app_name_lossy(raw);
    if is_valid_app_name(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_quote = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' | '\'' => {
                if in_quote == Some(c) {
                    in_quote = None;
                } else if in_quote.is_none() {
                    in_quote = Some(c);
                }
            }
            '#' if in_quote.is_none() => return &line[..i],
            _ => {}
        }
    }
    line
}

fn extract_toml_name(content: &str, target_table: &str) -> Option<String> {
    let mut in_target = false;
    for line in content.lines() {
        let trimmed = strip_toml_comment(line).trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            let is_match = trimmed
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .map(|s| s.trim() == target_table)
                .unwrap_or(false);
            if in_target && !is_match {
                break;
            }
            in_target = is_match;
            continue;
        }
        if in_target {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "name" {
                    let v = v.trim();
                    let val = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
                        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
                    {
                        &v[1..v.len() - 1]
                    } else {
                        v.trim_matches('"').trim_matches('\'')
                    };
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(prefix: &str) -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("skoci_det_{}_{}_{}", prefix, std::process::id(), id));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write_file(&self, rel: &str, content: &str) {
            let file_path = self.0.join(rel);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, content).unwrap();
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_suggest_app_name_package_json_over_dir() {
        let proj = TempProject::new("random_dir_name");
        proj.write_file("package.json", r#"{"name": "my-express-app"}"#);
        let suggested = ProjectDetector::suggest_app_name(proj.path());
        assert_eq!(suggested.as_deref(), Some("my-express-app"));
    }

    #[test]
    fn test_suggest_app_name_strips_scope() {
        let proj = TempProject::new("scoped_dir");
        proj.write_file("package.json", r#"{"name": "@my-scope/my-awesome-app"}"#);
        let suggested = ProjectDetector::suggest_app_name(proj.path());
        assert_eq!(suggested.as_deref(), Some("my-awesome-app"));
    }

    #[test]
    fn test_suggest_app_name_fallbacks() {
        let proj_cargo = TempProject::new("cargo_fallback");
        proj_cargo.write_file(
            "Cargo.toml",
            "[package]\nname = \"rust-backend\"\nversion = \"0.1.0\"\n",
        );
        assert_eq!(
            ProjectDetector::suggest_app_name(proj_cargo.path()).as_deref(),
            Some("rust-backend")
        );

        let proj_pyproject = TempProject::new("pyproject_fallback");
        proj_pyproject.write_file(
            "pyproject.toml",
            "[project]\nname = \"python-service\"\nversion = \"0.1.0\"\n",
        );
        assert_eq!(
            ProjectDetector::suggest_app_name(proj_pyproject.path()).as_deref(),
            Some("python-service")
        );

        let proj_poetry = TempProject::new("poetry_fallback");
        proj_poetry.write_file(
            "pyproject.toml",
            "[tool.poetry]\nname = \"poetry-service\"\nversion = \"0.1.0\"\n",
        );
        assert_eq!(
            ProjectDetector::suggest_app_name(proj_poetry.path()).as_deref(),
            Some("poetry-service")
        );

        let proj_precedence = TempProject::new("precedence");
        proj_precedence.write_file("package.json", r#"{"name": "from-package-json"}"#);
        proj_precedence.write_file("Cargo.toml", "[package]\nname = \"from-cargo\"\n");
        proj_precedence.write_file("pyproject.toml", "[project]\nname = \"from-python\"\n");
        assert_eq!(
            ProjectDetector::suggest_app_name(proj_precedence.path()).as_deref(),
            Some("from-package-json")
        );

        let proj_pkg_no_name = TempProject::new("pkg_no_name");
        proj_pkg_no_name.write_file("package.json", r#"{"version": "1.0.0"}"#);
        proj_pkg_no_name.write_file("Cargo.toml", "[package]\nname = \"from-cargo\"\n");
        assert_eq!(
            ProjectDetector::suggest_app_name(proj_pkg_no_name.path()).as_deref(),
            Some("from-cargo")
        );
    }

    #[test]
    fn test_suggest_app_name_invalid_names_return_none() {
        let proj_dots = TempProject::new("dots");
        proj_dots.write_file("package.json", r#"{"name": "..."}"#);
        assert_eq!(ProjectDetector::suggest_app_name(proj_dots.path()), None);

        let proj_empty = TempProject::new("empty_name");
        proj_empty.write_file("package.json", r#"{"name": ""}"#);
        assert_eq!(ProjectDetector::suggest_app_name(proj_empty.path()), None);

        let proj_dashes = TempProject::new("dashes");
        proj_dashes.write_file("Cargo.toml", "[package]\nname = \"---\"\n");
        assert_eq!(ProjectDetector::suggest_app_name(proj_dashes.path()), None);

        let proj_reserved = TempProject::new("reserved");
        proj_reserved.write_file("package.json", r#"{"name": "skoci"}"#);
        assert_eq!(ProjectDetector::suggest_app_name(proj_reserved.path()), None);

        let proj_none = TempProject::new("no_manifests");
        assert_eq!(ProjectDetector::suggest_app_name(proj_none.path()), None);
    }

    #[test]
    fn test_suggest_app_name_malformed_manifests_no_panic() {
        let proj_bad_json = TempProject::new("bad_json");
        proj_bad_json.write_file("package.json", "{ broken json: true ");
        assert_eq!(ProjectDetector::suggest_app_name(proj_bad_json.path()), None);

        let proj_trunc_json = TempProject::new("trunc_json");
        proj_trunc_json.write_file("package.json", r#"{"name": "#);
        assert_eq!(ProjectDetector::suggest_app_name(proj_trunc_json.path()), None);

        let proj_bad_toml = TempProject::new("bad_toml");
        proj_bad_toml.write_file("Cargo.toml", "[package\nname = \n");
        assert_eq!(ProjectDetector::suggest_app_name(proj_bad_toml.path()), None);

        let proj_bad_py = TempProject::new("bad_py");
        proj_bad_py.write_file("pyproject.toml", "[[[unclosed header\nname = ");
        assert_eq!(ProjectDetector::suggest_app_name(proj_bad_py.path()), None);
    }

    #[test]
    fn test_full_context_templates_remove_skoci_directory() {
        let d_static = Detection {
            framework: Framework::Static,
            port: 80,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_static = ProjectDetector::dockerfile(&d_static).unwrap();
        assert!(df_static.contains("COPY . /usr/share/nginx/html\nRUN rm -rf /usr/share/nginx/html/.skoci"));

        let d_node = Detection {
            framework: Framework::NodeServer,
            port: 3000,
            health: "/".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_node = ProjectDetector::dockerfile(&d_node).unwrap();
        assert!(df_node.contains("COPY . .\nRUN rm -rf /app/.skoci"));

        let d_py = Detection {
            framework: Framework::Python,
            port: 8000,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_py = ProjectDetector::dockerfile(&d_py).unwrap();
        assert!(df_py.contains("COPY . .\nRUN rm -rf /app/.skoci"));

        let d_php = Detection {
            framework: Framework::Php,
            port: 80,
            health: "/".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_php = ProjectDetector::dockerfile(&d_php).unwrap();
        assert!(df_php.contains("COPY . /var/www/html/\nRUN rm -rf /var/www/html/.skoci"));
    }

    #[test]
    fn test_unprivileged_users_and_runtime_safety() {
        let d_node = Detection {
            framework: Framework::NodeServer,
            port: 3000,
            health: "/".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_node = ProjectDetector::dockerfile(&d_node).unwrap();
        assert!(df_node.contains("RUN chown -R node:node /app\nUSER node"));

        let d_py = Detection {
            framework: Framework::Python,
            port: 8000,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_py = ProjectDetector::dockerfile(&d_py).unwrap();
        assert!(df_py.contains("RUN useradd -m -u 1000 appuser && chown -R appuser:appuser /app\nUSER appuser"));

        let d_rust = Detection {
            framework: Framework::Rust,
            port: 8080,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_rust = ProjectDetector::dockerfile(&d_rust).unwrap();
        assert!(df_rust.contains("RUN adduser -D -u 1000 appuser"));
        assert!(df_rust.contains("RUN chown -R appuser:appuser /app\nUSER appuser"));

        let d_go = Detection {
            framework: Framework::Go,
            port: 8080,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_go = ProjectDetector::dockerfile(&d_go).unwrap();
        assert!(df_go.contains("RUN adduser -D -u 1000 appuser"));
        assert!(df_go.contains("RUN chown -R appuser:appuser /app\nUSER appuser"));

        let d_static = Detection {
            framework: Framework::Static,
            port: 80,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_static = ProjectDetector::dockerfile(&d_static).unwrap();
        assert!(!df_static.contains("USER "));

        let d_static_build = Detection {
            framework: Framework::StaticBuild,
            port: 80,
            health: "/healthz".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_static_build = ProjectDetector::dockerfile(&d_static_build).unwrap();
        assert!(!df_static_build.contains("USER "));

        let d_php = Detection {
            framework: Framework::Php,
            port: 80,
            health: "/".to_string(),
            services: BTreeMap::new(),
            notes: Vec::new(),
        };
        let df_php = ProjectDetector::dockerfile(&d_php).unwrap();
        assert!(!df_php.contains("USER "));
    }
}
