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
    assert_eq!(d.port, 80);
    assert_eq!(d.health, "/healthz");
    assert!(d.services.is_empty());

    let dockerfile = ProjectDetector::dockerfile(&d).expect("dockerfile present");
    assert!(dockerfile.contains("FROM nginx:1.27-alpine"));
    assert!(dockerfile.contains("location = /healthz"));
    assert!(dockerfile.contains("try_files $uri $uri.html $uri/ =404;"));
    assert!(dockerfile.contains("chmod -R a+rX /usr/share/nginx/html"));

    let yaml = ProjectDetector::generate_skoci_yaml("my-site", &d);
    assert!(yaml.contains("app: my-site"));
    assert!(yaml.contains("port: 80"));
    assert!(!yaml.contains("example.com"));

    let parsed = ConfigLoader::parse_config(&yaml).expect("round-trip config parsing");
    assert_eq!(parsed.app, "my-site");
    assert_eq!(parsed.run.port, 80);
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
    assert_eq!(d.port, 80);
    assert_eq!(d.health, "/healthz");

    let dockerfile = ProjectDetector::dockerfile(&d).expect("dockerfile present");
    assert!(dockerfile.contains("FROM node:22-alpine AS builder"));
    assert!(dockerfile.contains("FROM nginx:1.27-alpine"));
    assert!(dockerfile.contains("/app/dist /usr/share/nginx/html"));
    assert!(dockerfile.contains("try_files $uri $uri.html $uri/ /index.html =404;"));
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
fn test_dockerignore() {
    let ignore = ProjectDetector::dockerignore();
    assert!(ignore.contains(".git\n"));
    assert!(ignore.contains(".env*"));
    assert!(ignore.contains(".skoci"));
    assert!(ignore.contains("node_modules"));
    assert!(ignore.contains("target"));
    assert!(ignore.contains("__pycache__"));
    assert!(ignore.contains("*.log"));
    assert!(ignore.contains(".DS_Store"));
    assert!(!ignore.contains("skoci.yaml"));
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
