//! Safe extraction of archives received from a container engine.
//!
//! `copy_from_container` streams a tar archive produced by the engine — which is
//! effectively untrusted, since a container can control its own filesystem. The
//! `tar` crate's `Archive::unpack` has some built-in guards, but this module
//! validates every entry explicitly before writing anything to the host, so a
//! hostile archive cannot escape the destination directory:
//!
//! - every path component must be a plain name — no `..`, no absolute/root/drive
//!   prefix, no other traversal;
//! - only regular files and directories are written — symlinks, hardlinks,
//!   character/block devices, and FIFOs are rejected (they are the classic
//!   link-escape and device vectors);
//! - Windows reserved device names (CON, NUL, COM1–9, LPT1–9, …) are rejected.

use std::path::{Component, Path};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("archive entry `{0}` has an unsafe path")]
    UnsafePath(String),
    #[error("archive entry `{0}` is a symlink, device, or other non-file entry")]
    UnsafeEntryType(String),
    #[error("archive entry `{0}` uses a reserved device name")]
    ReservedName(String),
    #[error("archive could not be read: {0}")]
    Io(String),
}

/// Extracts `archive_bytes` under `destination`, validating every entry first.
/// Any unsafe entry aborts the whole extraction with a typed error rather than
/// being silently skipped.
pub fn extract_safely(archive_bytes: &[u8], destination: &Path) -> Result<(), ExtractionError> {
    std::fs::create_dir_all(destination).map_err(|error| ExtractionError::Io(error.to_string()))?;

    let mut archive = tar::Archive::new(archive_bytes);
    let entries = archive
        .entries()
        .map_err(|error| ExtractionError::Io(error.to_string()))?;

    for entry in entries {
        let mut entry = entry.map_err(|error| ExtractionError::Io(error.to_string()))?;
        let path = entry
            .path()
            .map_err(|error| ExtractionError::Io(error.to_string()))?
            .into_owned();
        let display = path.to_string_lossy().into_owned();

        // Only regular files and directories are ever written. Symlinks,
        // hardlinks, and device/FIFO nodes are the link-escape / device vectors.
        if !matches!(
            entry.header().entry_type(),
            tar::EntryType::Regular | tar::EntryType::Directory
        ) {
            return Err(ExtractionError::UnsafeEntryType(display));
        }

        safe_relative_path(&path)?;

        let target = destination.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ExtractionError::Io(error.to_string()))?;
        }
        entry
            .unpack(&target)
            .map_err(|error| ExtractionError::Io(error.to_string()))?;
    }
    Ok(())
}

/// Validates that a tar entry path is a safe relative path to join under a
/// destination. Every component must be a plain name; `..`, absolute paths,
/// root, and Windows drive prefixes are all rejected, as are reserved device
/// names.
pub fn safe_relative_path(path: &Path) -> Result<(), ExtractionError> {
    let display = path.to_string_lossy().into_owned();
    let mut has_name = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                has_name = true;
                if is_reserved_windows_name(&part.to_string_lossy()) {
                    return Err(ExtractionError::ReservedName(display));
                }
            }
            // A leading/embedded "." is harmless and normalized away.
            Component::CurDir => {}
            // ParentDir ("..") escapes upward; RootDir / Prefix are absolute.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExtractionError::UnsafePath(display));
            }
        }
    }
    if has_name {
        Ok(())
    } else {
        Err(ExtractionError::UnsafePath(display))
    }
}

/// Whether `name` is a Windows reserved device name (case-insensitive, ignoring
/// any extension) — writing such a name can open a device instead of a file.
fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 {
        return stem
            .as_bytes()
            .get(3)
            .is_some_and(|digit| (b'1'..=b'9').contains(digit));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_with(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            // `append_data` writes the header + data; for a link type the link
            // name is the data path, but for this test the raw bytes suffice to
            // exercise the entry-type gate.
            header.set_cksum();
            let _ = builder.append_data(&mut header, name, *data);
        }
        builder.into_inner().unwrap_or_default()
    }

    /// Builds a raw ustar archive with a single regular-file entry whose name is
    /// written verbatim — bypassing the `tar::Builder`, which refuses to *write*
    /// traversal paths. A hostile engine, of course, can emit such bytes, which
    /// is exactly what the extractor must defend against.
    fn raw_tar(name: &str, data: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        let name_bytes = name.as_bytes();
        header[..name_bytes.len()].copy_from_slice(name_bytes);
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0'; // regular file
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        // Checksum is computed with its own field filled with spaces.
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());

        let mut out = header.to_vec();
        out.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        // Pad the data block, then two zero blocks marking end-of-archive.
        out.resize(out.len() + pad + 1024, 0);
        out
    }

    #[test]
    fn plain_relative_paths_are_allowed() {
        assert!(safe_relative_path(Path::new("file.txt")).is_ok());
        assert!(safe_relative_path(Path::new("dir/nested/file.txt")).is_ok());
        assert!(safe_relative_path(Path::new("./dir/file")).is_ok());
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        assert!(matches!(
            safe_relative_path(Path::new("../escape")),
            Err(ExtractionError::UnsafePath(_))
        ));
        assert!(matches!(
            safe_relative_path(Path::new("dir/../../escape")),
            Err(ExtractionError::UnsafePath(_))
        ));
        assert!(matches!(
            safe_relative_path(Path::new("/etc/passwd")),
            Err(ExtractionError::UnsafePath(_))
        ));
        #[cfg(windows)]
        assert!(matches!(
            safe_relative_path(Path::new(r"C:\Windows\system32")),
            Err(ExtractionError::UnsafePath(_))
        ));
    }

    #[test]
    fn reserved_device_names_are_rejected() {
        assert!(matches!(
            safe_relative_path(Path::new("NUL")),
            Err(ExtractionError::ReservedName(_))
        ));
        assert!(matches!(
            safe_relative_path(Path::new("dir/con.txt")),
            Err(ExtractionError::ReservedName(_))
        ));
        assert!(matches!(
            safe_relative_path(Path::new("COM1")),
            Err(ExtractionError::ReservedName(_))
        ));
        // Not reserved: a name that merely starts like one.
        assert!(safe_relative_path(Path::new("console.txt")).is_ok());
        assert!(safe_relative_path(Path::new("COM10")).is_ok());
    }

    #[test]
    fn empty_path_is_rejected() {
        assert!(matches!(
            safe_relative_path(Path::new("")),
            Err(ExtractionError::UnsafePath(_))
        ));
    }

    #[test]
    fn symlink_entries_are_rejected_by_extraction() {
        let dir = std::env::temp_dir().join(format!(
            "studio-extract-symlink-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let archive = tar_with(&[("link", tar::EntryType::Symlink, b"/etc/passwd")]);
        let result = extract_safely(&archive, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(ExtractionError::UnsafeEntryType(_))));
    }

    #[test]
    fn traversal_entries_are_rejected_by_extraction() {
        let dir = std::env::temp_dir().join(format!(
            "studio-extract-traversal-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let archive = raw_tar("../escape.txt", b"pwned");
        let result = extract_safely(&archive, &dir);
        // Confirm nothing landed outside the destination.
        let escaped = dir.parent().map(|parent| parent.join("escape.txt"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(ExtractionError::UnsafePath(_))));
        if let Some(escaped) = escaped {
            assert!(!escaped.exists());
        }
    }

    #[test]
    fn safe_nested_files_extract() {
        let dir = std::env::temp_dir().join(format!(
            "studio-extract-ok-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let archive = tar_with(&[("nested/hello.txt", tar::EntryType::Regular, b"hi")]);
        let result = extract_safely(&archive, &dir);
        let extracted = dir.join("nested/hello.txt");
        let exists = extracted.exists();
        let contents = std::fs::read(&extracted).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(exists);
        assert_eq!(contents, b"hi");
    }
}
