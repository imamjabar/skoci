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
}

impl Drop for PackedContext {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
    let out_file = File::create(&temp_path).map_err(|e| {
        SkociError::internal(
            format!("Failed to create temporary archive file: {e}"),
            "Check if the temporary directory is writable",
        )
    })?;

    let gz = GzEncoder::new(out_file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);

    let mut builder = WalkBuilder::new(dir);
    builder
        .hidden(false)
        .require_git(false)
        .add_custom_ignore_filename(".dockerignore")
        .filter_entry(|entry| {
            if entry.depth() > 0 {
                let name = entry.file_name();
                if name == ".git" || name == ".env.skoci" || name == ".skoci" {
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

    Ok(PackedContext {
        path: temp_path,
        files: files_count,
        bytes,
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
}
