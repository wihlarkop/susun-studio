//! Studio metadata backup and restore-validation.
//!
//! A backup is a tar archive of a **consistent** database snapshot (produced
//! with `VACUUM INTO`, never a raw copy of the live file) plus a versioned
//! `manifest.json` carrying app/schema/platform/time, per-entry SHA-256
//! checksums, and a redacted human-readable summary. Engine images/containers,
//! credentials, tokens, and raw endpoint secrets are explicitly out of scope.
//!
//! Restore itself is a process-boundary operation handled by the Tauri
//! supervisor (see the runtime-data-2 plan); this module only *validates and
//! previews* an archive without mutating any active data.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use turso::Database;

const KIND: &str = "studio_metadata_backup";
const MANIFEST_ENTRY: &str = "manifest.json";
const DATABASE_ENTRY: &str = "studio.db";
const CONTENT_CLASS: &str = "studio_metadata";
const CURRENT_MANIFEST_MAJOR: u16 = 1;
const CURRENT_MANIFEST_MINOR: u16 = 0;

/// Human-readable profile rows are capped so a huge database can't bloat the
/// manifest; the full data still lives in the snapshot.
const MAX_PROFILE_SUMMARIES: usize = 200;

// Restore-validation limits. These bound the work done on an untrusted archive
// before any mutation is considered. `MAX_ARCHIVE_BYTES` also sets the request
// body limit on the restore-preview route (see routes/mod.rs).
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

/// Categories deliberately excluded from every backup, surfaced in the manifest
/// and to the user so the boundary is explicit.
const EXCLUDED_CLASSES: &[&str] = &[
    "registry_credentials",
    "auth_tokens",
    "updater_keys",
    "raw_endpoint_secrets",
    "engine_images_containers_volumes",
];

/// Data the user must re-enter after a restore, since it is never in the backup.
const REENTER_AFTER_RESTORE: &[&str] = &[
    "Registry credentials",
    "Runtime endpoint secrets",
    "Updater signing keys",
    "External runtime configuration",
];

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("database error while building backup: {0}")]
    Database(#[from] turso::Error),
    #[error("filesystem error while building backup: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to serialize backup manifest: {0}")]
    Manifest(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("backup archive is larger than the {limit} byte limit")]
    ArchiveTooLarge { limit: u64 },
    #[error("backup archive could not be read: {0}")]
    InvalidArchive(String),
    #[error("backup archive contains an unexpected entry `{0}`")]
    UnexpectedEntry(String),
    #[error("backup archive contains an unsafe path `{0}`")]
    UnsafePath(String),
    #[error("backup archive entry `{name}` exceeds its {limit} byte limit")]
    EntryTooLarge { name: String, limit: u64 },
    #[error("backup archive is missing the `{0}` entry")]
    MissingEntry(&'static str),
    #[error("backup manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("this backup is not a Studio metadata backup")]
    WrongKind,
    #[error(
        "this backup uses manifest format v{found} but this app supports up to v{supported}; update Studio to restore it"
    )]
    IncompatibleManifest { found: u16, supported: u16 },
    #[error("backup checksum does not match; the archive is corrupt or was modified")]
    ChecksumMismatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ManifestVersion {
    major: u16,
    minor: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupPlatform {
    os: String,
    arch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupContentEntry {
    name: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeProfileSummary {
    display_name: String,
    runtime_class: String,
    ownership_state: String,
    availability_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupSummary {
    project_count: i64,
    job_count: i64,
    runtime_profile_count: i64,
    runtime_profiles: Vec<RuntimeProfileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupManifest {
    manifest_version: ManifestVersion,
    kind: String,
    app_version: String,
    schema_migration_version: i64,
    platform: BackupPlatform,
    created_at_epoch_seconds: u64,
    content_classes: Vec<String>,
    contents: Vec<BackupContentEntry>,
    excluded: Vec<String>,
    summary: BackupSummary,
}

/// Safe, non-mutating preview of an archive for the restore UI.
#[derive(Debug, Clone, Serialize)]
pub struct RestorePreview {
    /// Whether this app could restore the archive (schema not from the future).
    pub compatible: bool,
    /// Why it is incompatible, when it is.
    pub reason: Option<String>,
    pub manifest: RestoreManifestSummary,
    /// What a restore would replace.
    pub replacement_scope: Vec<String>,
    /// What the user must re-enter afterwards (never in the backup).
    pub reenter_after_restore: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreManifestSummary {
    pub app_version: String,
    pub schema_migration_version: i64,
    pub current_schema_migration_version: i64,
    pub platform_os: String,
    pub platform_arch: String,
    pub created_at_epoch_seconds: u64,
    pub project_count: i64,
    pub runtime_profile_count: i64,
    pub job_count: i64,
    pub content_classes: Vec<String>,
    pub excluded: Vec<String>,
}

/// Build a backup archive: a consistent snapshot of the database plus a
/// versioned, checksummed manifest, returned as tar bytes ready to be written
/// atomically by the caller.
pub async fn create_backup_archive(db: &Database, db_path: &Path) -> Result<Vec<u8>, BackupError> {
    let snapshot = SnapshotFile::create(db, db_path).await?;
    let db_bytes = std::fs::read(&snapshot.path)?;
    // The snapshot temp file is no longer needed once read into memory.
    drop(snapshot);

    let summary = collect_summary(db).await?;
    let manifest = BackupManifest {
        manifest_version: ManifestVersion {
            major: CURRENT_MANIFEST_MAJOR,
            minor: CURRENT_MANIFEST_MINOR,
        },
        kind: KIND.to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_migration_version: crate::db::latest_migration_version(),
        platform: BackupPlatform {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        },
        created_at_epoch_seconds: now_epoch_seconds(),
        content_classes: vec![CONTENT_CLASS.to_owned()],
        contents: vec![BackupContentEntry {
            name: DATABASE_ENTRY.to_owned(),
            sha256: sha256_hex(&db_bytes),
            size_bytes: db_bytes.len() as u64,
        }],
        excluded: EXCLUDED_CLASSES.iter().map(|s| (*s).to_owned()).collect(),
        summary,
    };

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    Ok(build_archive(&manifest_bytes, &db_bytes)?)
}

/// Validate an archive and produce a preview, without touching active data.
pub fn validate_restore_archive(
    archive: &[u8],
    current_schema_version: i64,
) -> Result<RestorePreview, RestoreError> {
    if archive.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(RestoreError::ArchiveTooLarge {
            limit: MAX_ARCHIVE_BYTES,
        });
    }

    let (manifest_bytes, db_bytes) = read_known_entries(archive)?;
    let manifest_bytes = manifest_bytes.ok_or(RestoreError::MissingEntry(MANIFEST_ENTRY))?;
    let db_bytes = db_bytes.ok_or(RestoreError::MissingEntry(DATABASE_ENTRY))?;

    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| RestoreError::InvalidManifest(error.to_string()))?;

    if manifest.manifest_version.major > CURRENT_MANIFEST_MAJOR {
        return Err(RestoreError::IncompatibleManifest {
            found: manifest.manifest_version.major,
            supported: CURRENT_MANIFEST_MAJOR,
        });
    }
    if manifest.kind != KIND {
        return Err(RestoreError::WrongKind);
    }

    let db_entry = manifest
        .contents
        .iter()
        .find(|entry| entry.name == DATABASE_ENTRY)
        .ok_or(RestoreError::InvalidManifest(
            "manifest does not describe the database entry".to_owned(),
        ))?;
    if db_entry.size_bytes != db_bytes.len() as u64 || db_entry.sha256 != sha256_hex(&db_bytes) {
        return Err(RestoreError::ChecksumMismatch);
    }

    // A backup from a newer app (higher schema) can't be migrated backwards.
    let compatible = manifest.schema_migration_version <= current_schema_version;
    let reason = (!compatible).then(|| {
        format!(
            "This backup was created by a newer Studio (schema v{}) than this app (schema v{}). Update Studio to restore it.",
            manifest.schema_migration_version, current_schema_version
        )
    });

    Ok(RestorePreview {
        compatible,
        reason,
        manifest: RestoreManifestSummary {
            app_version: manifest.app_version,
            schema_migration_version: manifest.schema_migration_version,
            current_schema_migration_version: current_schema_version,
            platform_os: manifest.platform.os,
            platform_arch: manifest.platform.arch,
            created_at_epoch_seconds: manifest.created_at_epoch_seconds,
            project_count: manifest.summary.project_count,
            runtime_profile_count: manifest.summary.runtime_profile_count,
            job_count: manifest.summary.job_count,
            content_classes: manifest.content_classes,
            excluded: manifest.excluded,
        },
        replacement_scope: vec![
            "All Studio metadata: projects, preferences, runtime profiles and bindings, plans, jobs, and history".to_owned(),
        ],
        reenter_after_restore: REENTER_AFTER_RESTORE.iter().map(|s| (*s).to_owned()).collect(),
    })
}

/// A `VACUUM INTO` snapshot that deletes its temp file on drop.
struct SnapshotFile {
    path: PathBuf,
}

impl SnapshotFile {
    async fn create(db: &Database, db_path: &Path) -> Result<Self, BackupError> {
        let path = snapshot_temp_path(db_path);
        let conn = db.connect()?;
        // turso needs the destination as a SQL literal. Forward slashes are
        // valid on Windows, and single quotes are doubled to stay inside the
        // literal. The path is daemon-generated, never user input.
        let literal = path
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''");
        conn.execute(&format!("VACUUM INTO '{literal}'"), ())
            .await?;
        Ok(Self { path })
    }
}

impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn snapshot_temp_path(db_path: &Path) -> PathBuf {
    let file_name = format!("studio-backup-{}.db", uuid::Uuid::new_v4().simple());
    // Prefer the database's own directory so the snapshot lands on the same
    // filesystem; fall back to the system temp dir.
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => std::env::temp_dir().join(file_name),
    }
}

async fn collect_summary(db: &Database) -> Result<BackupSummary, turso::Error> {
    let conn = db.connect()?;
    let project_count = scalar_count(&conn, "SELECT COUNT(*) FROM projects").await?;
    let job_count = scalar_count(&conn, "SELECT COUNT(*) FROM jobs").await?;
    let runtime_profile_count =
        scalar_count(&conn, "SELECT COUNT(*) FROM runtime_profiles").await?;

    let mut runtime_profiles = Vec::new();
    {
        let mut rows = conn
            .query(
                "SELECT display_name, runtime_class, ownership_state, availability_state
                 FROM runtime_profiles ORDER BY display_name ASC LIMIT ?1",
                turso::params![MAX_PROFILE_SUMMARIES as i64],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            runtime_profiles.push(RuntimeProfileSummary {
                display_name: row.get(0)?,
                runtime_class: row.get(1)?,
                ownership_state: row.get(2)?,
                availability_state: row.get(3)?,
            });
        }
    }

    Ok(BackupSummary {
        project_count,
        job_count,
        runtime_profile_count,
        runtime_profiles,
    })
}

async fn scalar_count(conn: &turso::Connection, sql: &str) -> Result<i64, turso::Error> {
    let mut rows = conn.query(sql, ()).await?;
    Ok(match rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    })
}

/// The manifest and database bytes read from an archive, each present or not.
type KnownEntries = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Read only the two entries a Studio backup may contain, enforcing shape,
/// path-safety, and per-entry size limits. Any other entry is rejected.
fn read_known_entries(archive: &[u8]) -> Result<KnownEntries, RestoreError> {
    let mut manifest_bytes = None;
    let mut db_bytes = None;

    let mut reader = tar::Archive::new(Cursor::new(archive));
    let entries = reader
        .entries()
        .map_err(|error| RestoreError::InvalidArchive(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| RestoreError::InvalidArchive(error.to_string()))?;
        let path = entry
            .path()
            .map_err(|error| RestoreError::InvalidArchive(error.to_string()))?
            .into_owned();

        let name = safe_entry_name(&path)?;
        let (limit, slot) = match name.as_str() {
            MANIFEST_ENTRY => (MAX_MANIFEST_BYTES, &mut manifest_bytes),
            DATABASE_ENTRY => (MAX_DATABASE_BYTES, &mut db_bytes),
            _ => return Err(RestoreError::UnexpectedEntry(name)),
        };

        let declared = entry.header().size().unwrap_or(u64::MAX);
        if declared > limit {
            return Err(RestoreError::EntryTooLarge { name, limit });
        }
        let mut buffer = Vec::new();
        entry
            .take(limit + 1)
            .read_to_end(&mut buffer)
            .map_err(|error| RestoreError::InvalidArchive(error.to_string()))?;
        if buffer.len() as u64 > limit {
            return Err(RestoreError::EntryTooLarge { name, limit });
        }
        *slot = Some(buffer);
    }

    Ok((manifest_bytes, db_bytes))
}

/// Reject absolute paths, parent-dir traversal, and anything that isn't a
/// single plain filename.
fn safe_entry_name(path: &Path) -> Result<String, RestoreError> {
    let mut components = path.components();
    let (Some(std::path::Component::Normal(name)), None) = (components.next(), components.next())
    else {
        return Err(RestoreError::UnsafePath(path.display().to_string()));
    };
    Ok(name.to_string_lossy().into_owned())
}

fn build_archive(manifest_bytes: &[u8], db_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut archive = tar::Builder::new(Vec::new());
    append_bytes(&mut archive, MANIFEST_ENTRY, manifest_bytes)?;
    append_bytes(&mut archive, DATABASE_ENTRY, db_bytes)?;
    archive.into_inner()
}

fn append_bytes(
    archive: &mut tar::Builder<Vec<u8>>,
    name: &str,
    contents: &[u8],
) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, name, contents)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
