//! Safe extraction of archives received from a container engine.
//!
//! `copy_from_container` streams a tar archive produced by the engine — which is
//! effectively untrusted, since a container can control its own filesystem. This
//! module validates the entire archive before writing anything, then extracts it
//! without ever following a symlink or reparse point, so a hostile archive
//! cannot escape the destination directory or leave partial state behind:
//!
//! - **Validate first, then write.** Every entry is validated and its contents
//!   buffered before any file is created, so a valid entry followed by an unsafe
//!   one leaves nothing on disk.
//! - **Safe paths.** Every path component must be a plain name — no `..`, no
//!   absolute/root/drive prefix; no Windows-forbidden character (`< > : " | ? *`
//!   or control chars, where `:` would open an NTFS alternate data stream); no
//!   trailing dot/space (Windows strips them, which can unmask a device name);
//!   and no reserved device name (CON, NUL, COM1–9, LPT1–9, …).
//! - **No link following.** The destination and every subdirectory are created
//!   one component at a time; an existing symlink/junction/reparse point in the
//!   path is rejected rather than traversed. Files are written with `fs::write`
//!   (never `tar`'s `unpack`), so no symlink is created or followed and no
//!   engine-controlled file mode is applied.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

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

fn io_error(error: impl std::fmt::Display) -> ExtractionError {
    ExtractionError::Io(error.to_string())
}

/// Extracts `archive_bytes` under `destination`, validating the whole archive
/// first. Any unsafe entry aborts the whole extraction with a typed error, and
/// nothing is written until validation has fully succeeded.
pub fn extract_safely(archive_bytes: &[u8], destination: &Path) -> Result<(), ExtractionError> {
    // Phase 1 — validate every entry and buffer its contents. No filesystem
    // writes happen here, so a hostile entry cannot leave partial state behind.
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    let mut archive = tar::Archive::new(archive_bytes);
    for entry in archive.entries().map_err(io_error)? {
        let mut entry = entry.map_err(io_error)?;
        let path = entry.path().map_err(io_error)?.into_owned();
        let display = path.to_string_lossy().into_owned();

        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                safe_relative_path(&path)?;
                directories.push(path);
            }
            tar::EntryType::Regular => {
                safe_relative_path(&path)?;
                let mut data = Vec::new();
                entry.read_to_end(&mut data).map_err(io_error)?;
                files.push((path, data));
            }
            // Symlinks, hardlinks, and device/FIFO nodes are the link-escape /
            // device vectors: only regular files and directories are ever written.
            _ => return Err(ExtractionError::UnsafeEntryType(display)),
        }
    }

    // Phase 2 — write into the destination, never following a link/junction.
    ensure_real_dir(destination)?;
    for directory in &directories {
        create_dir_no_follow(destination, directory)?;
    }
    for (path, data) in &files {
        if let Some(parent) = path.parent() {
            create_dir_no_follow(destination, parent)?;
        }
        let target = destination.join(path);
        // Refuse to write through a pre-existing link at the final path.
        if let Ok(meta) = std::fs::symlink_metadata(&target)
            && is_link_like(&meta)
        {
            return Err(ExtractionError::UnsafePath(target.display().to_string()));
        }
        std::fs::write(&target, data).map_err(io_error)?;
    }
    Ok(())
}

/// Validates that a tar entry path is a safe relative path to join under a
/// destination. Every component must be a plain name; traversal, absolute paths,
/// Windows-forbidden characters, trailing dots/spaces, and reserved device names
/// are all rejected.
pub fn safe_relative_path(path: &Path) -> Result<(), ExtractionError> {
    let display = path.to_string_lossy().into_owned();
    let mut has_name = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                has_name = true;
                validate_component(&part.to_string_lossy(), &display)?;
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

/// Validates a single path component's name. Applied on all platforms so a
/// container cannot smuggle a Windows-hostile name (device, ADS, trailing
/// dot/space) onto a Windows host regardless of where the daemon runs.
fn validate_component(name: &str, display: &str) -> Result<(), ExtractionError> {
    // Windows-forbidden characters. `:` would address an NTFS alternate data
    // stream; control characters are never legitimate in a copied filename.
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control())
    {
        return Err(ExtractionError::UnsafePath(display.to_owned()));
    }
    // Windows silently strips trailing dots and spaces, which can unmask a
    // reserved name (`NUL ` -> `NUL`) or collide with an existing path.
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(ExtractionError::UnsafePath(display.to_owned()));
    }
    if is_reserved_windows_name(name) {
        return Err(ExtractionError::ReservedName(display.to_owned()));
    }
    Ok(())
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

/// Ensures `path` is a real directory, creating it if missing. An existing
/// symlink/junction (or a non-directory) at `path` is rejected rather than used.
fn ensure_real_dir(path: &Path) -> Result<(), ExtractionError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if is_link_like(&meta) || !meta.is_dir() {
                Err(ExtractionError::UnsafePath(path.display().to_string()))
            } else {
                Ok(())
            }
        }
        Err(_) => std::fs::create_dir_all(path).map_err(io_error),
    }
}

/// Creates the directory chain `relative` under `root`, one component at a time,
/// rejecting any existing component that is a symlink/junction/reparse point (so
/// extraction can never traverse a pre-existing link out of `root`).
fn create_dir_no_follow(root: &Path, relative: &Path) -> Result<(), ExtractionError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            current.push(part);
            match std::fs::symlink_metadata(&current) {
                Ok(meta) => {
                    if is_link_like(&meta) || !meta.is_dir() {
                        return Err(ExtractionError::UnsafePath(current.display().to_string()));
                    }
                }
                Err(_) => std::fs::create_dir(&current).map_err(io_error)?,
            }
        }
    }
    Ok(())
}

/// Whether a file's metadata indicates a symlink or (on Windows) any reparse
/// point such as a junction/mount point.
fn is_link_like(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a raw ustar archive whose entry names are written verbatim,
    /// bypassing the `tar::Builder`, which refuses to *write* traversal paths — a
    /// hostile engine can, of course, emit such bytes.
    fn raw_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in entries {
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
            header[148..156].copy_from_slice(b"        ");
            let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
            header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(data);
            let pad = (512 - (data.len() % 512)) % 512;
            out.resize(out.len() + pad, 0);
        }
        out.resize(out.len() + 1024, 0); // two zero blocks mark end-of-archive
        out
    }

    fn unique_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "studio-extract-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ))
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
    fn windows_forbidden_chars_ads_and_trailing_are_rejected() {
        // `:` addresses an NTFS alternate data stream.
        assert!(matches!(
            safe_relative_path(Path::new("dir/file.txt:stream")),
            Err(ExtractionError::UnsafePath(_))
        ));
        // Other Windows-forbidden characters.
        for bad in ["a<b", "a>b", "a|b", "a?b", "a*b", "a\"b"] {
            assert!(
                matches!(
                    safe_relative_path(Path::new(bad)),
                    Err(ExtractionError::UnsafePath(_))
                ),
                "expected `{bad}` rejected"
            );
        }
        // Trailing space / dot (Windows strips them).
        assert!(matches!(
            safe_relative_path(Path::new("name ")),
            Err(ExtractionError::UnsafePath(_))
        ));
        assert!(matches!(
            safe_relative_path(Path::new("name.")),
            Err(ExtractionError::UnsafePath(_))
        ));
        // A trailing-space reserved name must not slip past the reserved check.
        assert!(safe_relative_path(Path::new("NUL ")).is_err());
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
        let dir = unique_dir("symlink-entry");
        let target = b"/etc/passwd";
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(target.len() as u64);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o644);
        header.set_cksum();
        let _ = builder.append_data(&mut header, "link", target.as_slice());
        let archive = builder.into_inner().unwrap_or_default();
        let result = extract_safely(&archive, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(ExtractionError::UnsafeEntryType(_))));
    }

    #[test]
    fn traversal_entries_are_rejected_by_extraction() {
        let dir = unique_dir("traversal");
        let archive = raw_tar(&[("../escape.txt", b"pwned")]);
        let result = extract_safely(&archive, &dir);
        let escaped = dir.parent().map(|parent| parent.join("escape.txt"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(ExtractionError::UnsafePath(_))));
        if let Some(escaped) = escaped {
            assert!(!escaped.exists());
        }
    }

    #[test]
    fn a_valid_entry_before_an_unsafe_one_leaves_nothing_written() {
        let dir = unique_dir("partial");
        // First a valid file, then a traversal entry: validation must fail before
        // the valid file is ever written.
        let archive = raw_tar(&[("good.txt", b"ok"), ("../evil.txt", b"pwned")]);
        let result = extract_safely(&archive, &dir);
        let good_exists = dir.join("good.txt").exists();
        let dir_exists = dir.exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(ExtractionError::UnsafePath(_))));
        assert!(!good_exists, "the earlier valid entry must not be written");
        assert!(
            !dir_exists,
            "the destination must not be created on failure"
        );
    }

    #[test]
    fn safe_nested_files_extract() {
        let dir = unique_dir("ok");
        let archive = raw_tar(&[("nested/hello.txt", b"hi")]);
        let result = extract_safely(&archive, &dir);
        let extracted = dir.join("nested/hello.txt");
        let exists = extracted.exists();
        let contents = std::fs::read(&extracted).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(exists);
        assert_eq!(contents, b"hi");
    }

    /// A pre-existing symlink inside the destination must not be followed: a
    /// `nested/…` entry must not write through `dest/nested -> outside`.
    #[cfg(unix)]
    #[test]
    fn existing_symlink_in_destination_is_not_followed() {
        use std::os::unix::fs::symlink;
        let base = unique_dir("symdest");
        let dest = base.join("dest");
        let outside = base.join("outside");
        let _ = std::fs::create_dir_all(&dest);
        let _ = std::fs::create_dir_all(&outside);
        let _ = symlink(&outside, dest.join("nested"));

        let archive = raw_tar(&[("nested/pwned.txt", b"x")]);
        let result = extract_safely(&archive, &dest);
        let escaped = outside.join("pwned.txt").exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(
            matches!(result, Err(ExtractionError::UnsafePath(_))),
            "got {result:?}"
        );
        assert!(!escaped, "extraction escaped through the symlink");
    }
}
