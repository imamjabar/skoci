use skoci_api::models::DatabaseEngine;
use skoci_core::config::{
    is_safe_relative_path, is_valid_app_name, is_valid_env_key, is_valid_hostname, ConfigLoader,
};
use skoci_core::detector::{Framework, ProjectDetector};

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("skoci_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn write_file(&self, relative: &str, content: &str) {
        let target = self.0.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, content).unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_detect_static_html() {
    let td = TestDir::new();
    td.write_file("index.html", "<!doctype html><html><body>Hi</body></html>");

    let d = ProjectDetector::detect(td.path());
    assert_eq!(d.framework, Framework::Static);
    assert_eq!(d.port, 8080);
    assert_eq!(d.health, "/healthz");
    assert!(d.services.is_empty());

    let dockerfile = ProjectDetector::dockerfile(&d).expect("dockerfile present");
    assert!(dockerfile.contains("FROM nginxinc/nginx-unprivileged:1.27-alpine"));
    assert!(dockerfile.contains("USER root"));
    assert!(dockerfile.contains("listen 8080;"));
    assert!(dockerfile.contains("location = /healthz"));
    assert!(dockerfile.contains("try_files $uri $uri.html $uri/ =404;"));
    assert!(dockerfile.contains("chmod -R a+rX /usr/share/nginx/html"));
    assert!(dockerfile.contains("EXPOSE 8080"));
    assert!(dockerfile.contains("USER nginx"));

    let yaml = ProjectDetector::generate_skoci_yaml("my-site", &d);
    assert!(yaml.contains("app: my-site"));
    assert!(yaml.contains("port: 8080"));
    assert!(!yaml.contains("example.com"));

    let parsed = ConfigLoader::parse_config(&yaml).expect("round-trip config parsing");
    assert_eq!(parsed.app, "my-site");
    assert_eq!(parsed.run.port, 8080);
    assert_eq!(parsed.run.health, "/healthz");
    assert!(parsed.services.is_empty());
}

#[test]
fn test_detect_node_server() {
    let td = TestDir::new();
    td.write_file(
        "package.json",
        r#"{"name": "express-app", "scripts": {"start": "node index.js"}}"#,
    );

    let d = ProjectDetector::detect(td.path());
    assert_eq!(d.framework, Framework::NodeServer);
    assert_eq!(d.port, 3000);
    assert_eq!(d.health, "/");

    let dockerfile = ProjectDetector::dockerfile(&d).expect("dockerfile present");
    assert!(dockerfile.contains("FROM node:22-alpine"));
    assert!(dockerfile.contains("npm install"));
    assert!(dockerfile.contains("ENV PORT=3000 HOST=0.0.0.0"));
    assert!(dockerfile.contains(r#"CMD ["npm", "start"]"#));

    // With lockfile, uses npm ci
    td.write_file("package-lock.json", r#"{"lockfileVersion": 3}"#);
    let d2 = ProjectDetector::detect(td.path());
    let dockerfile2 = ProjectDetector::dockerfile(&d2).expect("dockerfile present");
    assert!(dockerfile2.contains("npm ci"));
}

#[test]
fn test_detect_static_build() {
    let td = TestDir::new();
    td.write_file(
        "package.json",
        r#"{"name": "vite-app", "scripts": {"build": "vite build"}}"#,
    );
    td.write_file("dist/index.html", "<h1>App</h1>");

    let d = ProjectDetector::detect(td.path());
    assert_eq!(d.framework, Framework::StaticBuild);
    assert_eq!(d.port, 8080);
    assert_eq!(d.health, "/healthz");

    let dockerfile = ProjectDetector::dockerfile(&d).expect("dockerfile present");
    assert!(dockerfile.contains("FROM node:22-alpine AS builder"));
    assert!(dockerfile.contains("FROM nginxinc/nginx-unprivileged:1.27-alpine"));
    assert!(dockerfile.contains("USER root"));
    assert!(dockerfile.contains("listen 8080;"));
    assert!(dockerfile.contains("/app/dist /usr/share/nginx/html"));
    assert!(dockerfile.contains("RUN rm -rf /usr/share/nginx/html/.skoci"));
    assert!(dockerfile.contains("try_files $uri $uri.html $uri/ /index.html =404;"));
    assert!(dockerfile.contains("EXPOSE 8080"));
    assert!(dockerfile.contains("USER nginx"));
}

#[test]
fn test_static_and_static_build_deny_rules() {
    let td_static = TestDir::new();
    td_static.write_file("index.html", "<!doctype html><html><body>Hi</body></html>");
    let d_static = ProjectDetector::detect(td_static.path());
    let df_static = ProjectDetector::dockerfile(&d_static).expect("static dockerfile");

    let td_build = TestDir::new();
    td_build.write_file(
        "package.json",
        r#"{"name": "vite-app", "scripts": {"build": "vite build"}}"#,
    );
    td_build.write_file("dist/index.html", "<h1>App</h1>");
    let d_build = ProjectDetector::detect(td_build.path());
    let df_build = ProjectDetector::dockerfile(&d_build).expect("static build dockerfile");

    for df in [&df_static, &df_build] {
        let ext_rule = r"location ~* \\.(md|markdown|py|sh|bash|rb|pl|sql|db|sqlite|sqlite3|toml|ini|lock|bak|log|yaml|yml|env)$ {";
        let manifest_rule = r"location ~* /(package\\.json|package-lock\\.json|yarn\\.lock|pnpm-lock\\.yaml|composer\\.json|composer\\.lock|requirements\\.txt|Makefile|Dockerfile[^/]*)$ {";
        let catchall = "location / {";

        let ext_idx = df.find(ext_rule).expect("extension deny rule present");
        let manifest_idx = df.find(manifest_rule).expect("manifest deny rule present");
        let catchall_idx = df.find(catchall).expect("catch-all location present");
        assert!(ext_idx < catchall_idx);
        assert!(manifest_idx < catchall_idx);

        // Data JSON paths (e.g. /data/items.json) must not be denied by a blanket *.json rule
        assert!(!df.contains("|json|"));
        assert!(!df.contains("(json|"));
        assert!(!df.contains("|json)"));
    }
}

#[test]
fn test_detect_services_postgres_and_cache() {
    let td = TestDir::new();
    td.write_file(
        "package.json",
        r#"{
            "name": "api",
            "scripts": {"start": "node index.js"},
            "dependencies": {
                "pg": "^8.11.0",
                "redis": "^4.6.0"
            }
        }"#,
    );

    let d = ProjectDetector::detect(td.path());
    assert!(d.services.contains_key("db"));
    assert!(d.services.contains_key("cache"));

    let db = &d.services["db"];
    assert_eq!(db.engine, DatabaseEngine::Postgres);
    assert_eq!(db.version, "16");
    let backup = db.backup.as_ref().expect("backup policy present");
    assert_eq!(backup.schedule, "0 2 * * *");
    assert_eq!(backup.keep, 7);

    let cache = &d.services["cache"];
    assert_eq!(cache.engine, DatabaseEngine::Valkey);
    assert_eq!(cache.version, "8");

    let yaml = ProjectDetector::generate_skoci_yaml("belajar-pg", &d);
    assert!(yaml.contains("services:"));
    assert!(yaml.contains("engine: postgres"));
    assert!(yaml.contains("engine: valkey"));

    let parsed = ConfigLoader::parse_config(&yaml).expect("round-trip config parsing");
    assert_eq!(parsed.services.len(), 2);
}

#[test]
fn test_detect_plain_html_has_no_services() {
    let td = TestDir::new();
    td.write_file("index.html", "<p>Simple</p>");

    let d = ProjectDetector::detect(td.path());
    assert!(d.services.is_empty());

    let yaml = ProjectDetector::generate_skoci_yaml("plain-html", &d);
    assert!(!yaml.contains("services:"));

    let parsed = ConfigLoader::parse_config(&yaml).expect("round-trip config parsing");
    assert!(parsed.services.is_empty());
}

#[test]
fn test_validation_rejects_underscore_app_names() {
    assert!(!is_valid_app_name("my_app"));
    assert!(!is_valid_app_name("belajar_paper"));
    assert!(is_valid_app_name("belajar-paper"));

    let yaml = "version: 1\napp: belajar_paper\nrun:\n  port: 8080\n";
    let err = ConfigLoader::parse_config(yaml).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.message.contains("underscore"));
    assert!(err.fix.contains("belajar-paper"));
}

#[test]
fn test_validation_rejects_bad_hostnames() {
    assert!(!is_valid_hostname("localhost"));
    assert!(!is_valid_hostname("127.0.0.1"));
    assert!(!is_valid_hostname("::1"));
    assert!(!is_valid_hostname("-bad.com"));
    assert!(!is_valid_hostname("bad-.com"));
    assert!(!is_valid_hostname("Bad.com"));
    assert!(!is_valid_hostname(""));
    assert!(is_valid_hostname("good.example.com"));

    let yaml = "version: 1\napp: test-app\nrun:\n  port: 8080\ndomains:\n  - 127.0.0.1\n";
    let err = ConfigLoader::parse_config(yaml).unwrap_err();
    assert_eq!(err.code, "config_invalid");
}

#[test]
fn test_validation_rejects_duplicate_domains_in_same_list() {
    let yaml_domains = "version: 1\napp: test-app\nrun:\n  port: 8080\ndomains:\n  - app.example.com\n  - app.example.com\n";
    let err_domains = ConfigLoader::parse_config(yaml_domains).unwrap_err();
    assert_eq!(err_domains.code, "config_invalid");
    assert_eq!(
        err_domains.message,
        "Domain 'app.example.com' is listed twice under 'domains'"
    );
    assert_eq!(err_domains.fix, "Remove the repeated entry");

    let yaml_preview = "version: 1\napp: test-app\nrun:\n  port: 8080\npreview_domains:\n  - preview.example.com\n  - preview.example.com\n";
    let err_preview = ConfigLoader::parse_config(yaml_preview).unwrap_err();
    assert_eq!(err_preview.code, "config_invalid");
    assert_eq!(
        err_preview.message,
        "Domain 'preview.example.com' is listed twice under 'preview_domains'"
    );
    assert_eq!(err_preview.fix, "Remove the repeated entry");
}

#[test]
fn test_validation_rejects_duplicate_domains_across_lists() {
    let yaml = "version: 1\napp: test-app\nrun:\n  port: 8080\ndomains:\n  - app.example.com\npreview_domains:\n  - app.example.com\n";
    let err = ConfigLoader::parse_config(yaml).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert_eq!(
        err.message,
        "Domain 'app.example.com' is listed under both 'domains' and 'preview_domains'"
    );
    assert!(err.fix.contains("One hostname routes to exactly one environment"));
    assert!(err.fix.contains("<app>-preview.<base-domain>"));
    assert!(err.fix.contains("preview_domains"));
}

#[test]
fn test_validation_rejects_parent_dir_dockerfile() {
    assert!(!is_safe_relative_path("../Dockerfile"));
    assert!(!is_safe_relative_path("/etc/Dockerfile"));
    assert!(is_safe_relative_path("Dockerfile"));
    assert!(is_safe_relative_path("docker/Dockerfile"));

    let yaml = "version: 1\napp: test-app\nbuild:\n  dockerfile: ../Dockerfile\nrun:\n  port: 8080\n";
    let err = ConfigLoader::parse_config(yaml).unwrap_err();
    assert_eq!(err.code, "config_invalid");
}

#[test]
fn test_validation_rejects_skoci_public_key() {
    assert!(!is_valid_env_key("bad-key"));
    assert!(is_valid_env_key("GOOD_KEY"));

    let yaml = "version: 1\napp: test-app\nrun:\n  port: 8080\nenv:\n  public:\n    SKOCI_KEY: val\n";
    let err = ConfigLoader::parse_config(yaml).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.message.contains("SKOCI_"));
}

#[test]
fn test_upload_ignore() {
    let ignore = ProjectDetector::upload_ignore();
    assert!(ignore.contains(".git\n"));
    assert!(ignore.contains(".env*"));
    assert!(ignore.contains("*.pem"));
    assert!(ignore.contains("*.key"));
    assert!(ignore.contains("id_rsa*"));
    assert!(ignore.contains("id_ed25519*"));
    assert!(ignore.contains(".npmrc"));
    assert!(ignore.contains(".pypirc"));
    assert!(ignore.contains(".netrc"));
    assert!(ignore.contains(".aws"));
    assert!(ignore.contains(".ssh"));
    assert!(ignore.contains("node_modules"));
    assert!(ignore.contains("target"));
    assert!(ignore.contains("__pycache__"));
    assert!(ignore.contains("*.log"));
    assert!(ignore.contains(".DS_Store"));

    // Dropped patterns that previously broke deployments (D1)
    assert!(!ignore.contains(".skoci"));
    assert!(!ignore.contains("skoci.yaml"));
    assert!(!ignore.contains("Dockerfile"));
    assert!(!ignore.contains(".dockerignore"));
}

#[test]
fn test_locate_config_resolution_order() {
    let td = TestDir::new();
    assert_eq!(ConfigLoader::locate_config(td.path()), None);

    // Legacy root skoci.yaml is no longer resolved
    td.write_file("skoci.yaml", "version: 1\napp: legacy\nrun:\n  port: 8080\n");
    assert_eq!(ConfigLoader::locate_config(td.path()), None);

    // Only .skoci/skoci.yaml is resolved
    td.write_file(".skoci/skoci.yaml", "version: 1\napp: modern\nrun:\n  port: 8080\n");
    assert_eq!(
        ConfigLoader::locate_config(td.path()),
        Some(td.path().join(".skoci/skoci.yaml"))
    );
}

#[test]
fn test_framework_names() {
    assert_eq!(Framework::NodeServer.name(), "Node.js server");
    assert_eq!(Framework::Static.name(), "Static (nginx)");
    assert_eq!(Framework::StaticBuild.name(), "Static build (Node + nginx)");
    assert_eq!(Framework::Python.name(), "Python");
    assert_eq!(Framework::Rust.name(), "Rust");
    assert_eq!(Framework::Go.name(), "Go");
    assert_eq!(Framework::Php.name(), "PHP");
    assert_eq!(Framework::Docker.name(), "Docker");
}
