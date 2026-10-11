use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Framework {
    Node,
    Python,
    Rust,
    Go,
    Laravel,
    DockerGeneric,
}

impl Framework {
    pub fn name(&self) -> &'static str {
        match self {
            Framework::Node => "Node.js",
            Framework::Python => "Python",
            Framework::Rust => "Rust",
            Framework::Go => "Go",
            Framework::Laravel => "Laravel/PHP",
            Framework::DockerGeneric => "Docker",
        }
    }

    pub fn default_port(&self) -> u16 {
        match self {
            Framework::Node => 3000,
            Framework::Python => 8000,
            Framework::Rust => 8080,
            Framework::Go => 8080,
            Framework::Laravel => 80,
            Framework::DockerGeneric => 8080,
        }
    }

    pub fn default_dockerfile(&self) -> &'static str {
        match self {
            Framework::Node => {
                r#"# Slim Node multi-stage build
FROM node:20-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build --if-present

FROM node:20-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
COPY --from=builder /app ./
EXPOSE 3000
CMD ["npm", "start"]
"#
            }
            Framework::Python => {
                r#"# Slim Python build
FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY . .
EXPOSE 8000
CMD ["python", "main.py"]
"#
            }
            Framework::Rust => {
                r#"# Slim Rust multi-stage build
FROM rust:1.80-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:3.20 AS runner
WORKDIR /app
COPY --from=builder /app/target/release/* /app/server
EXPOSE 8080
CMD ["/app/server"]
"#
            }
            Framework::Go => {
                r#"# Slim Go multi-stage build
FROM golang:1.22-alpine AS builder
WORKDIR /app
COPY . .
RUN CGO_ENABLED=0 go build -o server .

FROM alpine:3.20 AS runner
WORKDIR /app
COPY --from=builder /app/server ./
EXPOSE 8080
CMD ["./server"]
"#
            }
            Framework::Laravel => {
                r#"# Slim PHP / Laravel build
FROM php:8.3-cli-alpine
WORKDIR /app
RUN docker-php-ext-install pdo pdo_mysql
COPY . .
EXPOSE 80
CMD ["php", "-S", "0.0.0.0:80", "-t", "public"]
"#
            }
            Framework::DockerGeneric => {
                r#"FROM alpine:3.20
WORKDIR /app
COPY . .
EXPOSE 8080
CMD ["./run.sh"]
"#
            }
        }
    }
}

pub struct ProjectDetector;

impl ProjectDetector {
    pub fn detect(dir: impl AsRef<Path>) -> Framework {
        let dir = dir.as_ref();

        if dir.join("Cargo.toml").exists() {
            Framework::Rust
        } else if dir.join("go.mod").exists() {
            Framework::Go
        } else if dir.join("package.json").exists() {
            Framework::Node
        } else if dir.join("requirements.txt").exists() || dir.join("pyproject.toml").exists() {
            Framework::Python
        } else if dir.join("composer.json").exists() {
            Framework::Laravel
        } else {
            Framework::DockerGeneric
        }
    }

    pub fn generate_skoci_yaml(app_name: &str, framework: &Framework) -> String {
        let port = framework.default_port();
        format!(
            r#"version: 1
app: {app_name}
server: default
build:
  mode: server
  dockerfile: Dockerfile
run:
  port: {port}
  health: /healthz
  memory: 256m
domains:
  - {app_name}.example.com
env:
  public:
    - PORT
  secrets:
    - DATABASE_URL
services:
  db:
    engine: postgres
    version: "16"
    backup:
      schedule: "0 2 * * *"
      keep: 7
      to: local
"#
        )
    }

    pub fn generate_policy_yaml() -> String {
        r#"version: 1
preview:
  deploy: auto
  rollback: auto
  db_restore: approve
  delete: approve
production:
  deploy: approve
  rollback: auto
  db_restore: approve
  delete: approve
"#
        .to_string()
    }
}
