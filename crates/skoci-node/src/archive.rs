use flate2::read::GzDecoder;
use skoci_api::error::{SkociError, SkociResult};
use std::fs::{self, File};
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
                format!("Gagal membuat direktori tujuan ekstraksi: {e}"),
                "Periksa izin folder tujuan",
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
            format!("Gagal kanonikalisasi direktori tujuan: {e}"),
            "Periksa path direktori tujuan",
        )
    })?;

    let file = File::open(src).map_err(|e| {
        SkociError::bad_request(
            format!("Gagal membuka file arsip tar.gz: {e}"),
            "Pastikan file arsip ada dan dapat dibaca",
        )
    })?;

    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    let mut total_unpacked: u64 = 0;

    let entries = archive.entries().map_err(|e| {
        SkociError::bad_request(
            format!("Gagal membaca entri tar: {e}"),
            "Pastikan file merupakan format .tar.gz yang valid",
        )
    })?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            SkociError::bad_request(
                format!("Format entri tar rusak: {e}"),
                "Periksa integritas arsip tar",
            )
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| {
                SkociError::bad_request(
                    format!("Path dalam tar tidak valid: {e}"),
                    "Periksa struktur path di dalam arsip",
                )
            })?
            .into_owned();

        if entry_path.is_absolute() {
            return Err(SkociError::bad_request(
                format!("Path absolut tidak diizinkan: {}", entry_path.display()),
                "Gunakan path relatif di dalam arsip",
            ));
        }

        if entry_path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(SkociError::bad_request(
                format!("Path traversal ('..') tidak diizinkan: {}", entry_path.display()),
                "Hapus traversal '..' dari path di dalam arsip",
            ));
        }

        let entry_type = entry.header().entry_type();
        let raw_mode = entry.header().mode().unwrap_or(0o644);

        match entry_type {
            EntryType::Char | EntryType::Block | EntryType::Fifo => {
                return Err(SkociError::bad_request(
                    format!("Tipe file khusus ({:?}) tidak diizinkan di dalam arsip", entry_type),
                    "Hapus device / fifo node dari arsip",
                ));
            }
            _ => {}
        }

        let target_path = dest_canonical.join(&entry_path);

        if entry_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|e| {
                SkociError::bad_request(
                    format!("Gagal membuat subdirektori: {e}"),
                    "Periksa izin penulisan",
                )
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(0o755));
            }
        } else if entry_type.is_symlink() || entry_type.is_hard_link() {
            let link_target = entry
                .link_name()
                .map_err(|e| {
                    SkociError::bad_request(
                        format!("Gagal membaca link target: {e}"),
                        "Periksa symlink dalam arsip",
                    )
                })?
                .ok_or_else(|| {
                    SkociError::bad_request("Symlink tidak memiliki target", "Periksa integritas symlink")
                })?
                .into_owned();

            if link_target.is_absolute() {
                return Err(SkociError::bad_request(
                    format!("Target link absolut tidak diizinkan: {}", link_target.display()),
                    "Gunakan target relatif di dalam arsip",
                ));
            }

            let parent_dir = target_path.parent().unwrap_or(&dest_canonical);
            let combined = parent_dir.join(&link_target);
            let normalized = normalize_lexical_path(&combined);

            if !normalized.starts_with(&dest_canonical) {
                return Err(SkociError::bad_request(
                    format!(
                        "Target link meloloskan diri dari folder ekstraksi: {}",
                        link_target.display()
                    ),
                    "Pastikan semua symlink tetap berada di dalam folder proyek",
                ));
            }

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    SkociError::bad_request(
                        format!("Gagal membuat folder parent untuk link: {e}"),
                        "Periksa izin folder",
                    )
                })?;
            }

            if entry_type.is_symlink() {
                #[cfg(unix)]
                {
                    if target_path.is_symlink() || target_path.exists() {
                        let _ = fs::remove_file(&target_path);
                    }
                    std::os::unix::fs::symlink(&link_target, &target_path).map_err(|e| {
                        SkociError::bad_request(
                            format!("Gagal membuat symlink: {e}"),
                            "Periksa izin pembuatan symlink",
                        )
                    })?;
                }
            } else if entry_type.is_hard_link() {
                if target_path.exists() {
                    let _ = fs::remove_file(&target_path);
                }
                fs::hard_link(&normalized, &target_path).map_err(|e| {
                    SkociError::bad_request(
                        format!("Gagal membuat hardlink: {e}"),
                        "Periksa target file untuk hardlink",
                    )
                })?;
            }
        } else if entry_type.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    SkociError::bad_request(
                        format!("Gagal membuat folder parent untuk file: {e}"),
                        "Periksa izin penulisan",
                    )
                })?;
            }

            let mut out_file = File::create(&target_path).map_err(|e| {
                SkociError::bad_request(
                    format!("Gagal membuat file tujuan {}: {e}", target_path.display()),
                    "Periksa izin penulisan",
                )
            })?;

            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf).map_err(|e| {
                    let _ = fs::remove_file(&target_path);
                    SkociError::bad_request(format!("Gagal membaca isi file: {e}"), "Coba lagi")
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
                    SkociError::bad_request(format!("Gagal menulis file: {e}"), "Cek disk space")
                })?;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let is_exec = (raw_mode & 0o111) != 0;
                let mode = if is_exec {
                    (raw_mode & 0o755) | 0o700
                } else {
                    (raw_mode & 0o644) | 0o600
                };
                let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(mode));
            }
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
        assert!(err.message.contains("tidak diizinkan"));

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
        assert!(err.message.contains("absolut"));

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
        assert!(err.message.contains("meloloskan diri"));

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
}
