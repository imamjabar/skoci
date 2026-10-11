use flate2::write::GzEncoder;
use flate2::Compression;
use ignore::WalkBuilder;
use skoci_api::error::{SkociError, SkociResult};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct PackedContext {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub excluded_sensitive: Vec<String>,
}

impl Drop for PackedContext {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn is_sensitive_entry(name: &str, is_dir: bool) -> bool {
    if is_dir && (name == ".aws" || name == ".ssh" || name == ".docker" || name == ".gnupg") {
        return true;
    }
    if name == ".aws" || name == ".ssh" || name == ".docker" || name == ".gnupg" {
        return true;
    }
    if name == ".npmrc" || name == ".pypirc" || name == ".netrc" || name == ".git-credentials" {
        return true;
    }
    if name.ends_with(".pem") || name.ends_with(".key") {
        return true;
    }
    if name.starts_with("id_rsa") || name.starts_with("id_ed25519") || name.starts_with("id_ecdsa") {
        return true;
    }
    if name == ".env" {
        return true;
    }
    if let Some(rest) = name.strip_prefix(".env.") {
        if rest != "example" && rest != "sample" && rest != "template" {
            return true;
        }
    }
    false
}

pub fn pack_context(dir: impl AsRef<Path>) -> SkociResult<PackedContext> {
    let dir = dir.as_ref();
    let skoci_yaml = dir.join("skoci.yaml");
    if !skoci_yaml.exists() {
        return Err(SkociError::config_invalid(
            "skoci.yaml file not found in project directory",
            "Run `skoci init` to create skoci.yaml first",
        ));
    }

    let temp_path = std::env::temp_dir().join(format!("skoci-pack-{}.tar.gz", uuid::Uuid::new_v4()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let out_file = opts.open(&temp_path).map_err(|e| {
        SkociError::internal(
            format!("Failed to create temporary archive file: {e}"),
            "Check if the temporary directory is writable",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = out_file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }

    let gz = GzEncoder::new(out_file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);

    let excluded_sensitive = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let excluded_clone = std::sync::Arc::clone(&excluded_sensitive);
    let base_dir = dir.to_path_buf();

    let mut builder = WalkBuilder::new(dir);
    builder
        .hidden(false)
        .require_git(false)
        .add_custom_ignore_filename(".dockerignore")
        .filter_entry(move |entry| {
            if entry.depth() > 0 {
                let name = entry.file_name().to_string_lossy();
                if name == ".git" || name == ".env.skoci" || name == ".skoci" {
                    return false;
                }
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                if is_sensitive_entry(&name, is_dir) {
                    if let Ok(rel) = entry.path().strip_prefix(&base_dir) {
                        let rel_str = rel
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/");
                    excluded_clone.lock().push(rel_str);
                    }
                    return false;
                }
            }
            true
        });
    let mut files_count = 0usize;

    for entry in builder.build() {
        let entry = entry.map_err(|e| {
            SkociError::bad_request(
                format!("Failed to scan project files: {e}"),
                "Check file access permissions in the project directory",
            )
        })?;

        if entry.depth() == 0 {
            continue;
        }

        let path = entry.path();
        let rel_path = path.strip_prefix(dir).map_err(|e| {
            SkociError::internal(format!("Failed to compute relative path: {e}"), "Try deploying again")
        })?;

        let rel_str = rel_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");

        let metadata = std::fs::symlink_metadata(path).map_err(|e| {
            SkociError::internal(
                format!("Failed to read metadata for {}: {e}", path.display()),
                "Check project file permissions",
            )
        })?;

        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        let _ = header.set_username("");
        let _ = header.set_groupname("");

        if metadata.file_type().is_symlink() {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            let target = std::fs::read_link(path).map_err(|e| {
                SkociError::bad_request(
                    format!("Failed to read symlink {}: {e}", path.display()),
                    "Ensure symlink is valid",
                )
            })?;
            tar.append_link(&mut header, &rel_str, &target).map_err(|e| {
                SkociError::internal(format!("Failed to append symlink to archive: {e}"), "Try deploying again")
            })?;
            files_count += 1;
        } else if metadata.is_dir() {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            let dir_name = if rel_str.ends_with('/') {
                rel_str
            } else {
                format!("{rel_str}/")
            };
            tar.append_data(&mut header, dir_name, std::io::empty()).map_err(|e| {
                SkociError::internal(format!("Failed to append directory to archive: {e}"), "Try deploying again")
            })?;
        } else {
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(metadata.len());

            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o100 != 0 {
                    0o755
                } else {
                    0o644
                }
            };
            #[cfg(not(unix))]
            let mode = 0o644;

            header.set_mode(mode);
            let mut file = File::open(path).map_err(|e| {
                SkociError::bad_request(
                    format!("Failed to open file {}: {e}", path.display()),
                    "Ensure the file is readable",
                )
            })?;
            tar.append_data(&mut header, &rel_str, &mut file).map_err(|e| {
                SkociError::internal(format!("Failed to write tar archive: {e}"), "Try deploying again")
            })?;
            files_count += 1;
        }
    }

    let gz = tar.into_inner().map_err(|e| {
        SkociError::internal(format!("Failed to finalize tar archive: {e}"), "Try deploying again")
    })?;

    let file = gz.finish().map_err(|e| {
        SkociError::internal(format!("Failed to compress gzip archive: {e}"), "Try deploying again")
    })?;

    let bytes = file.metadata().map_err(|e| {
        SkociError::internal(format!("Failed to check archive size: {e}"), "Try deploying again")
    })?.len();

    let mut excluded = excluded_sensitive.lock().clone();
    excluded.sort();
    excluded.dedup();

    Ok(PackedContext {
        path: temp_path,
        files: files_count,
        bytes,
        excluded_sensitive: excluded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::collections::HashMap;

    #[test]
    fn test_skoci_yaml_missing() {
        let temp = std::env::temp_dir().join(format!("skoci-test-noyaml-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let res = pack_context(&temp);
        let _ = std::fs::remove_dir_all(&temp);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, "config_invalid");
        assert!(err.fix.contains("skoci init"));
    }

    #[test]
    fn test_pack_filters_and_modes() {
        let temp = std::env::temp_dir().join(format!("skoci-test-pack-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        std::fs::write(temp.join("skoci.yaml"), "app: my-app\nversion: 1\n").unwrap();
        std::fs::write(temp.join(".gitignore"), "git_ignored.txt\n").unwrap();
        std::fs::write(temp.join(".dockerignore"), "docker_ignored.txt\n").unwrap();
        std::fs::write(temp.join("git_ignored.txt"), "should be ignored").unwrap();
        std::fs::write(temp.join("docker_ignored.txt"), "should be ignored").unwrap();
        std::fs::write(temp.join(".env.skoci"), "SKOCI_TOKEN=secret").unwrap();

        let git_dir = temp.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), "[core]").unwrap();

        let script = temp.join("run.sh");
        std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();

        let private_file = temp.join("private.txt");
        std::fs::write(&private_file, "secret data").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&private_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let packed = pack_context(&temp).unwrap();
        let archive_path = packed.path.clone();
        assert!(archive_path.exists());
        assert!(packed.files >= 3);
        assert!(packed.bytes > 0);

        let file = File::open(&archive_path).unwrap();
        let gz = GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);

        let mut entries_found = HashMap::new();
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().to_string();
            let mode = entry.header().mode().unwrap() & 0o777;
            let uid = entry.header().uid().unwrap();
            let gid = entry.header().gid().unwrap();
            let mtime = entry.header().mtime().unwrap();
            assert_eq!(uid, 0);
            assert_eq!(gid, 0);
            assert_eq!(mtime, 0);
            entries_found.insert(path, mode);
        }

        assert!(entries_found.contains_key("skoci.yaml"));
        assert!(!entries_found.contains_key("git_ignored.txt"));
        assert!(!entries_found.contains_key("docker_ignored.txt"));
        assert!(!entries_found.contains_key(".env.skoci"));
        assert!(!entries_found.contains_key(".git/config"));
        assert!(!entries_found.contains_key(".git"));

        #[cfg(unix)]
        {
            assert_eq!(*entries_found.get("run.sh").unwrap(), 0o755);
            assert_eq!(*entries_found.get("private.txt").unwrap(), 0o644);
        }

        drop(packed);
        assert!(!archive_path.exists(), "Temp archive should be deleted on drop");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_sensitive_exclusions_and_archive_mode() {
        let temp = std::env::temp_dir().join(format!("skoci-test-sensitive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        std::fs::write(temp.join("skoci.yaml"), "app: secure-app\nversion: 1\n").unwrap();

        // Sensitive files that must be excluded
        std::fs::write(temp.join(".env"), "SECRET=1").unwrap();
        std::fs::write(temp.join(".env.production"), "SECRET=2").unwrap();
        std::fs::write(temp.join(".env.local"), "SECRET=3").unwrap();
        std::fs::write(temp.join("cert.pem"), "cert").unwrap();
        std::fs::write(temp.join("app.key"), "key").unwrap();
        std::fs::write(temp.join("id_rsa"), "privkey").unwrap();
        std::fs::write(temp.join("id_rsa.pub"), "pubkey").unwrap();
        std::fs::write(temp.join("id_ed25519"), "privkey").unwrap();
        std::fs::write(temp.join("id_ecdsa"), "privkey").unwrap();
        std::fs::write(temp.join(".npmrc"), "//registry=token").unwrap();
        std::fs::write(temp.join(".pypirc"), "token").unwrap();
        std::fs::write(temp.join(".netrc"), "token").unwrap();
        std::fs::write(temp.join(".git-credentials"), "creds").unwrap();

        // Sensitive directories
        let aws_dir = temp.join(".aws");
        std::fs::create_dir_all(&aws_dir).unwrap();
        std::fs::write(aws_dir.join("credentials"), "secret").unwrap();

        let ssh_dir = temp.join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        std::fs::write(ssh_dir.join("id_rsa"), "secret").unwrap();

        let docker_dir = temp.join(".docker");
        std::fs::create_dir_all(&docker_dir).unwrap();
        std::fs::write(docker_dir.join("config.json"), "secret").unwrap();

        let gnupg_dir = temp.join(".gnupg");
        std::fs::create_dir_all(&gnupg_dir).unwrap();
        std::fs::write(gnupg_dir.join("secring.gpg"), "secret").unwrap();

        // Safe example/template env files that MUST be included
        std::fs::write(temp.join(".env.example"), "FOO=BAR").unwrap();
        std::fs::write(temp.join(".env.sample"), "FOO=BAR").unwrap();
        std::fs::write(temp.join(".env.template"), "FOO=BAR").unwrap();

        let packed = pack_context(&temp).unwrap();
        let archive_path = packed.path.clone();
        assert!(archive_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = archive_path.metadata().unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "Archive must be created mode 0600");
        }

        assert!(packed.excluded_sensitive.contains(&".env".to_string()));
        assert!(packed.excluded_sensitive.contains(&".env.production".to_string()));
        assert!(packed.excluded_sensitive.contains(&".env.local".to_string()));
        assert!(packed.excluded_sensitive.contains(&"cert.pem".to_string()));
        assert!(packed.excluded_sensitive.contains(&"app.key".to_string()));
        assert!(packed.excluded_sensitive.contains(&"id_rsa".to_string()));
        assert!(packed.excluded_sensitive.contains(&"id_rsa.pub".to_string()));
        assert!(packed.excluded_sensitive.contains(&"id_ed25519".to_string()));
        assert!(packed.excluded_sensitive.contains(&"id_ecdsa".to_string()));
        assert!(packed.excluded_sensitive.contains(&".npmrc".to_string()));
        assert!(packed.excluded_sensitive.contains(&".pypirc".to_string()));
        assert!(packed.excluded_sensitive.contains(&".netrc".to_string()));
        assert!(packed.excluded_sensitive.contains(&".git-credentials".to_string()));
        assert!(packed.excluded_sensitive.contains(&".aws".to_string()));
        assert!(packed.excluded_sensitive.contains(&".ssh".to_string()));
        assert!(packed.excluded_sensitive.contains(&".docker".to_string()));
        assert!(packed.excluded_sensitive.contains(&".gnupg".to_string()));

        // Verify archive contents
        let file = File::open(&archive_path).unwrap();
        let gz = GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);

        let mut entries = Vec::new();
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            entries.push(entry.path().unwrap().to_string_lossy().to_string());
        }

        // Allowed templates must be present
        assert!(entries.contains(&".env.example".to_string()));
        assert!(entries.contains(&".env.sample".to_string()));
        assert!(entries.contains(&".env.template".to_string()));

        // Excluded files must NOT be in the archive
        assert!(!entries.contains(&".env".to_string()));
        assert!(!entries.contains(&".env.production".to_string()));
        assert!(!entries.contains(&".env.local".to_string()));
        assert!(!entries.contains(&"cert.pem".to_string()));
        assert!(!entries.contains(&"app.key".to_string()));
        assert!(!entries.contains(&"id_rsa".to_string()));
        assert!(!entries.contains(&".aws".to_string()));
        assert!(!entries.contains(&".aws/credentials".to_string()));
        assert!(!entries.contains(&".ssh".to_string()));
        assert!(!entries.contains(&".ssh/id_rsa".to_string()));

        drop(packed);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
