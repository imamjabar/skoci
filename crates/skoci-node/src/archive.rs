use flate2::read::GzDecoder;
use skoci_api::error::{SkociError, SkociResult};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use tar::{Archive, EntryType};

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            Component::Normal(part) => {
                out.push(part);
            }
            Component::RootDir | Component::Prefix(_) => {
                out.push(c.as_os_str());
            }
        }
    }
    out
}

/// Extract a .tar.gz file into `dest` (created). Rejects absolute paths, `..`, device/fifo entries, and
/// symlinks whose target escapes `dest`. Fails with payload_too_large when unpacked bytes exceed the limit.
pub fn extract_tar_gz(src: &Path, dest: &Path, max_unpacked_bytes: u64) -> SkociResult<u64> {
    if !dest.exists() {
        fs::create_dir_all(dest).map_err(|e| {
            SkociError::bad_request(
                format!("Failed to create extraction destination directory: {e}"),
                "Check destination directory permissions",
            )
        })?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o755));
    }

    let dest_canonical = dest.canonicalize().map_err(|e| {
        SkociError::bad_request(
            format!("Failed to canonicalize destination directory: {e}"),
            "Check destination directory path",
        )
    })?;

    let file = File::open(src).map_err(|e| {
        SkociError::bad_request(
            format!("Failed to open tar.gz archive file: {e}"),
            "Ensure archive file exists and is readable",
        )
    })?;

    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    let mut total_unpacked: u64 = 0;

    let entries = archive.entries().map_err(|e| {
        SkociError::bad_request(
            format!("Failed to read tar entries: {e}"),
            "Ensure file is a valid .tar.gz format",
        )
    })?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            SkociError::bad_request(
                format!("Corrupted tar entry format: {e}"),
                "Check tar archive integrity",
            )
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| {
                SkociError::bad_request(
                    format!("Invalid path in tar: {e}"),
                    "Check path structure in archive",
                )
            })?
            .into_owned();

        if entry_path.is_absolute() {
            return Err(SkociError::bad_request(
                format!("Absolute paths not allowed: {}", entry_path.display()),
                "Use relative paths inside the archive",
            ));
        }

        if entry_path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(SkociError::bad_request(
                format!("Path traversal ('..') not allowed: {}", entry_path.display()),
                "Remove '..' traversal from path inside the archive",
            ));
        }

        let entry_type = entry.header().entry_type();
        let raw_mode = entry.header().mode().unwrap_or(0o644);

        if entry_type.is_hard_link() {
            return Err(SkociError::bad_request(
                "Hard links are not allowed in archive",
                "Use regular files or symbolic links",
            ));
        }

        match entry_type {
            EntryType::Char | EntryType::Block | EntryType::Fifo => {
                return Err(SkociError::bad_request(
                    format!("Special file type ({:?}) not allowed in archive", entry_type),
                    "Remove device / fifo nodes from archive",
                ));
            }
            _ => {}
        }

        let mut clean_rel_path = PathBuf::new();
        for comp in entry_path.components() {
            match comp {
                Component::Normal(part) => clean_rel_path.push(part),
                Component::CurDir => {}
                _ => {}
            }
        }

        if clean_rel_path.as_os_str().is_empty() {
            continue;
        }

        let target_path = dest_canonical.join(&clean_rel_path);

        // Verify all ancestor directories between dest_canonical and target_path.
        // Every existing ancestor directory must be a real directory (symlink_metadata, not a symlink).
        // Never create or write through a symlinked parent.
        let mut current_ancestor = dest_canonical.clone();
        if let Some(parent) = clean_rel_path.parent() {
            for comp in parent.components() {
                if let Component::Normal(part) = comp {
                    current_ancestor.push(part);
                    match fs::symlink_metadata(&current_ancestor) {
                        Ok(meta) => {
                            if meta.file_type().is_symlink() || !meta.is_dir() {
                                return Err(SkociError::bad_request(
                                    format!(
                                        "Ancestor path '{}' is a symlink or not a directory",
                                        current_ancestor.display()
                                    ),
                                    "Ensure archive does not traverse through symlinks",
                                ));
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            fs::create_dir(&current_ancestor).map_err(|err| {
                                SkociError::bad_request(
                                    format!(
                                        "Failed to create ancestor directory '{}': {err}",
                                        current_ancestor.display()
                                    ),
                                    "Check directory permissions",
                                )
                            })?;
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = fs::set_permissions(
                                    &current_ancestor,
                                    fs::Permissions::from_mode(0o755),
                                );
                            }
                        }
                        Err(e) => {
                            return Err(SkociError::bad_request(
                                format!(
                                    "Failed to check ancestor path '{}': {e}",
                                    current_ancestor.display()
                                ),
                                "Check directory permissions",
                            ));
                        }
                    }
                }
            }
        }

        // If the final path already exists, reject (duplicate entry) unless it is a plain directory entry
        match fs::symlink_metadata(&target_path) {
            Ok(meta) => {
                if entry_type.is_dir() && meta.is_dir() && !meta.file_type().is_symlink() {
                    // Plain directory entry that already exists, allowed
                } else {
                    return Err(SkociError::bad_request(
                        format!("Duplicate or conflicting entry: {}", clean_rel_path.display()),
                        "Ensure archive does not contain duplicate entries",
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(SkociError::bad_request(
                    format!("Failed to check entry path '{}': {e}", target_path.display()),
                    "Check directory permissions",
                ));
            }
        }

        if entry_type.is_dir() {
            if !target_path.exists() {
                fs::create_dir(&target_path).map_err(|e| {
                    SkociError::bad_request(
                        format!("Failed to create directory: {e}"),
                        "Check write permissions",
                    )
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(0o755));
                }
            }
        } else if entry_type.is_symlink() {
            let link_target = entry
                .link_name()
                .map_err(|e| {
                    SkociError::bad_request(
                        format!("Failed to read link target: {e}"),
                        "Check symlinks in archive",
                    )
                })?
                .ok_or_else(|| {
                    SkociError::bad_request("Symlink has no target", "Check symlink integrity")
                })?
                .into_owned();

            if link_target.is_absolute() {
                return Err(SkociError::bad_request(
                    format!("Absolute link targets not allowed: {}", link_target.display()),
                    "Use relative targets inside the archive",
                ));
            }

            let parent_dir = target_path.parent().unwrap_or(&dest_canonical);
            let combined = parent_dir.join(&link_target);
            let normalized = normalize_lexical_path(&combined);

            if !normalized.starts_with(&dest_canonical) {
                return Err(SkociError::bad_request(
                    format!(
                        "Link target escapes extraction directory: {}",
                        link_target.display()
                    ),
                    "Ensure all symlinks stay inside the project directory",
                ));
            }

            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&link_target, &target_path).map_err(|e| {
                    SkociError::bad_request(
                        format!("Failed to create symlink: {e}"),
                        "Check symlink creation permissions",
                    )
                })?;
            }
        } else if entry_type.is_file() {
            #[cfg(unix)]
            let is_exec = (raw_mode & 0o111) != 0;
            #[cfg(unix)]
            let mode = if is_exec {
                (raw_mode & 0o755) | 0o700
            } else {
                (raw_mode & 0o644) | 0o600
            };

            #[cfg(unix)]
            let open_options = {
                use std::os::unix::fs::OpenOptionsExt;
                let mut opts = OpenOptions::new();
                opts.write(true).create_new(true).mode(mode);
                opts
            };
            #[cfg(not(unix))]
            let open_options = {
                let mut opts = OpenOptions::new();
                opts.write(true).create_new(true);
                opts
            };

            let mut out_file = open_options.open(&target_path).map_err(|e| {
                SkociError::bad_request(
                    format!("Failed to create destination file {}: {e}", target_path.display()),
                    "Check write permissions",
                )
            })?;

            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf).map_err(|e| {
                    let _ = fs::remove_file(&target_path);
                    SkociError::bad_request(format!("Failed to read file contents: {e}"), "Try again")
                })?;
                if n == 0 {
                    break;
                }
                total_unpacked += n as u64;
                if total_unpacked > max_unpacked_bytes {
                    let _ = fs::remove_file(&target_path);
                    let limit_mb = (max_unpacked_bytes / (1024 * 1024)).max(1);
                    return Err(SkociError::payload_too_large(limit_mb));
                }
                use std::io::Write;
                out_file.write_all(&buf[..n]).map_err(|e| {
                    let _ = fs::remove_file(&target_path);
                    SkociError::bad_request(format!("Failed to write file: {e}"), "Check disk space")
                })?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(mode));
            }
        } else {
            return Err(SkociError::bad_request(
                format!("Unsupported archive entry type {:?}", entry_type),
                "Only files, directories, and relative symlinks are supported",
            ));
        }
    }

    Ok(total_unpacked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder, Header};

    fn create_test_archive(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut gz);
            for &(path, data, is_exec) in entries {
                let mut header = Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(if is_exec { 0o755 } else { 0o644 });
                header.set_entry_type(EntryType::Regular);
                let header_bytes = header.as_mut_bytes();
                let path_bytes = path.as_bytes();
                header_bytes[..path_bytes.len()].copy_from_slice(path_bytes);
                header.set_cksum();
                tar.append(&header, data).unwrap();
            }
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn test_normal_archive_extracted_with_content_check() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_arch_normal_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("sample.tar.gz");
        let archive_bytes = create_test_archive(&[
            ("index.js", b"console.log('hello');", false),
            ("run.sh", b"#!/bin/sh\necho ok", true),
        ]);
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let bytes = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap();
        assert!(bytes > 0);

        let content = fs::read_to_string(dest.join("index.js")).unwrap();
        assert_eq!(content, "console.log('hello');");

        let script = fs::read_to_string(dest.join("run.sh")).unwrap();
        assert_eq!(script, "#!/bin/sh\necho ok");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dest.join("run.sh")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_traversal_rejected() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_arch_trav_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("trav.tar.gz");
        let archive_bytes = create_test_archive(&[("foo/../../escape.txt", b"escaped!", false)]);
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("not allowed"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_absolute_rejected() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_arch_abs_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("abs.tar.gz");
        let archive_bytes = create_test_archive(&[("/etc/danger.txt", b"danger", false)]);
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("Absolute"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_escaping_symlink_rejected() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_arch_symlink_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("sym.tar.gz");
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut gz);
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
            header.set_cksum();
            tar.append_link(&mut header, "bad_link", "../../escaped").unwrap();
            tar.finish().unwrap();
        }
        let archive_bytes = gz.finish().unwrap();
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("escapes"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_size_limit_enforced() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_arch_limit_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let big_data = vec![b'A'; 10_000];
        let tar_gz_path = temp_dir.join("big.tar.gz");
        let archive_bytes = create_test_archive(&[("big.txt", &big_data, false)]);
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 5_000).unwrap_err();
        assert_eq!(err.code, "payload_too_large");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_chained_symlink_escape_fails() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_chained_symlink_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("chained.tar.gz");
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut gz);

            // 1. Directory d1
            let mut h_dir = Header::new_gnu();
            h_dir.set_entry_type(EntryType::Directory);
            h_dir.set_size(0);
            h_dir.set_mode(0o755);
            h_dir.set_cksum();
            tar.append_data(&mut h_dir, "d1/", std::io::empty()).unwrap();

            // 2. Symlink d1/l1 -> ..
            let mut h_l1 = Header::new_gnu();
            h_l1.set_entry_type(EntryType::Symlink);
            h_l1.set_size(0);
            h_l1.set_cksum();
            tar.append_link(&mut h_l1, "d1/l1", "..").unwrap();

            // 3. Symlink l2 -> d1/l1/..
            let mut h_l2 = Header::new_gnu();
            h_l2.set_entry_type(EntryType::Symlink);
            h_l2.set_size(0);
            h_l2.set_cksum();
            tar.append_link(&mut h_l2, "l2", "d1/l1/..").unwrap();

            // 4. File l2/escape.txt (attempts to write through escaped symlink l2)
            let mut h_file = Header::new_gnu();
            let payload = b"pwned";
            h_file.set_size(payload.len() as u64);
            h_file.set_mode(0o644);
            h_file.set_entry_type(EntryType::Regular);
            let header_bytes = h_file.as_mut_bytes();
            let path_bytes = b"l2/escape.txt";
            header_bytes[..path_bytes.len()].copy_from_slice(path_bytes);
            h_file.set_cksum();
            tar.append(&h_file, &payload[..]).unwrap();

            tar.finish().unwrap();
        }
        let archive_bytes = gz.finish().unwrap();
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("symlink") || err.message.contains("not a directory"));

        // Verify escaped file was NOT created
        assert!(!temp_dir.join("escape.txt").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_normal_in_tree_symlink_extracts() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_normal_symlink_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("normal_sym.tar.gz");
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut gz);

            // 1. Regular file hello.txt
            let mut h_file = Header::new_gnu();
            let payload = b"hello from file";
            h_file.set_size(payload.len() as u64);
            h_file.set_mode(0o644);
            h_file.set_entry_type(EntryType::Regular);
            let header_bytes = h_file.as_mut_bytes();
            let path_bytes = b"hello.txt";
            header_bytes[..path_bytes.len()].copy_from_slice(path_bytes);
            h_file.set_cksum();
            tar.append(&h_file, &payload[..]).unwrap();

            // 2. In-tree symlink link.txt -> hello.txt
            let mut h_link = Header::new_gnu();
            h_link.set_entry_type(EntryType::Symlink);
            h_link.set_size(0);
            h_link.set_cksum();
            tar.append_link(&mut h_link, "link.txt", "hello.txt").unwrap();

            tar.finish().unwrap();
        }
        let archive_bytes = gz.finish().unwrap();
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let bytes = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap();
        assert!(bytes > 0);

        #[cfg(unix)]
        {
            let link_path = dest.join("link.txt");
            assert!(link_path.is_symlink());
            let target = fs::read_link(&link_path).unwrap();
            assert_eq!(target.to_str().unwrap(), "hello.txt");
            let content = fs::read_to_string(&link_path).unwrap();
            assert_eq!(content, "hello from file");
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hard_links_rejected() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_hard_link_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("hard_link.tar.gz");
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut gz);

            let mut h_file = Header::new_gnu();
            let payload = b"hello";
            h_file.set_size(payload.len() as u64);
            h_file.set_mode(0o644);
            h_file.set_entry_type(EntryType::Regular);
            let header_bytes = h_file.as_mut_bytes();
            let path_bytes = b"orig.txt";
            header_bytes[..path_bytes.len()].copy_from_slice(path_bytes);
            h_file.set_cksum();
            tar.append(&h_file, &payload[..]).unwrap();

            let mut h_hl = Header::new_gnu();
            h_hl.set_entry_type(EntryType::Link);
            h_hl.set_size(0);
            h_hl.set_cksum();
            tar.append_link(&mut h_hl, "hard.txt", "orig.txt").unwrap();

            tar.finish().unwrap();
        }
        let archive_bytes = gz.finish().unwrap();
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("Hard links are not allowed"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_duplicate_entry_rejected() {
        let temp_dir = std::env::temp_dir().join(format!("skoci_test_dup_{:x}", rand::random::<u64>()));
        let dest = temp_dir.join("extracted");
        fs::create_dir_all(&temp_dir).unwrap();

        let tar_gz_path = temp_dir.join("dup.tar.gz");
        let archive_bytes = create_test_archive(&[
            ("hello.txt", b"version 1", false),
            ("hello.txt", b"version 2", false),
        ]);
        fs::write(&tar_gz_path, archive_bytes).unwrap();

        let err = extract_tar_gz(&tar_gz_path, &dest, 1024 * 1024).unwrap_err();
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("Duplicate or conflicting entry"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
