//! Backup/restore-validation tests. Uses a file-backed turso database so the
//! `VACUUM INTO` snapshot path is exercised for real.

use super::{
    BackupContentEntry, BackupManifest, BackupPlatform, BackupSummary, CURRENT_MANIFEST_MAJOR,
    CURRENT_MANIFEST_MINOR, DATABASE_ENTRY, KIND, ManifestVersion, RestoreError, append_bytes,
    create_backup_archive, sha256_hex, validate_restore_archive,
};
use crate::db;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn unique_db_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("studio-backup-test-{}.db", uuid::Uuid::new_v4().simple()))
}

async fn seeded_db() -> TestResult<(turso::Database, std::path::PathBuf)> {
    let path = unique_db_path();
    let db = db::open_database(path.clone()).await?;
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms) VALUES ('p','Proj','/proj',1)",
        (),
    )
    .await?;
    conn.execute(
        "INSERT INTO runtime_profiles (id, provider_id, provider_runtime_key, display_name,
            product, platform, installation_state, process_state, connection_state,
            observed_at_ms, created_at_ms, updated_at_ms)
         VALUES ('rp','windows-podman','machine/a','Podman machine a','podman','windows',
            'installed','running','summarized', 1, 1, 1)",
        (),
    )
    .await?;
    Ok((db, path))
}

fn archive_from(entries: &[(&str, &[u8])]) -> TestResult<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in entries {
        append_bytes(&mut builder, name, bytes)?;
    }
    Ok(builder.into_inner()?)
}

fn manifest_json(schema_version: i64, db_bytes: &[u8]) -> TestResult<Vec<u8>> {
    let manifest = BackupManifest {
        manifest_version: ManifestVersion {
            major: CURRENT_MANIFEST_MAJOR,
            minor: CURRENT_MANIFEST_MINOR,
        },
        kind: KIND.to_owned(),
        app_version: "0.0.0-test".to_owned(),
        schema_migration_version: schema_version,
        platform: BackupPlatform {
            os: "windows".to_owned(),
            arch: "x86_64".to_owned(),
        },
        created_at_epoch_seconds: 1,
        content_classes: vec!["studio_metadata".to_owned()],
        contents: vec![BackupContentEntry {
            name: DATABASE_ENTRY.to_owned(),
            sha256: sha256_hex(db_bytes),
            size_bytes: db_bytes.len() as u64,
        }],
        excluded: vec!["registry_credentials".to_owned()],
        summary: BackupSummary {
            project_count: 0,
            job_count: 0,
            runtime_profile_count: 0,
            runtime_profiles: Vec::new(),
        },
    };
    Ok(serde_json::to_vec(&manifest)?)
}

#[tokio::test]
async fn roundtrip_backup_validates_and_summarizes() -> TestResult {
    let (db, path) = seeded_db().await?;
    let archive = create_backup_archive(&db, &path).await?;

    let current = db::latest_migration_version();
    let preview = validate_restore_archive(&archive, current)?;

    assert!(preview.compatible);
    assert!(preview.reason.is_none());
    assert_eq!(preview.manifest.project_count, 1);
    assert_eq!(preview.manifest.runtime_profile_count, 1);
    assert_eq!(preview.manifest.schema_migration_version, current);
    // The exclusion boundary and re-entry guidance are surfaced.
    assert!(preview.manifest.excluded.iter().any(|c| c == "registry_credentials"));
    assert!(!preview.reenter_after_restore.is_empty());

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tokio::test]
async fn tampered_database_fails_checksum() -> TestResult {
    let db_bytes = b"not-a-real-db-but-checksum-still-applies".to_vec();
    let manifest = manifest_json(db::latest_migration_version(), &db_bytes)?;
    // Same length so the size check passes but the content (and hash) differs.
    let mut tampered = db_bytes.clone();
    *tampered.last_mut().unwrap() ^= 0xff;
    let archive = archive_from(&[("manifest.json", &manifest), (DATABASE_ENTRY, &tampered)])?;

    let error = validate_restore_archive(&archive, db::latest_migration_version()).unwrap_err();
    assert!(matches!(error, RestoreError::ChecksumMismatch));
    Ok(())
}

#[tokio::test]
async fn future_schema_is_reported_incompatible() -> TestResult {
    let db_bytes = b"db".to_vec();
    let current = db::latest_migration_version();
    let manifest = manifest_json(current + 5, &db_bytes)?;
    let archive = archive_from(&[("manifest.json", &manifest), (DATABASE_ENTRY, &db_bytes)])?;

    let preview = validate_restore_archive(&archive, current)?;
    assert!(!preview.compatible);
    assert!(preview.reason.is_some());
    Ok(())
}

#[test]
fn safe_entry_name_rejects_traversal_and_absolute() {
    use std::path::Path;
    // The tar writer refuses to emit `..` paths, so the traversal guard is
    // verified directly — this is the function the archive reader relies on.
    assert_eq!(
        super::safe_entry_name(Path::new("manifest.json")).unwrap(),
        "manifest.json"
    );
    assert!(matches!(
        super::safe_entry_name(Path::new("../evil.txt")),
        Err(RestoreError::UnsafePath(_))
    ));
    assert!(matches!(
        super::safe_entry_name(Path::new("nested/child")),
        Err(RestoreError::UnsafePath(_))
    ));
    #[cfg(windows)]
    let absolute = Path::new(r"C:\Windows\evil");
    #[cfg(not(windows))]
    let absolute = Path::new("/etc/evil");
    assert!(matches!(
        super::safe_entry_name(absolute),
        Err(RestoreError::UnsafePath(_))
    ));
}

#[tokio::test]
async fn unexpected_entry_is_rejected() -> TestResult {
    let db_bytes = b"db".to_vec();
    let manifest = manifest_json(db::latest_migration_version(), &db_bytes)?;
    let archive = archive_from(&[
        ("manifest.json", &manifest),
        (DATABASE_ENTRY, &db_bytes),
        ("surprise.txt", b"hi"),
    ])?;

    let error = validate_restore_archive(&archive, db::latest_migration_version()).unwrap_err();
    assert!(matches!(error, RestoreError::UnexpectedEntry(_)));
    Ok(())
}

#[tokio::test]
async fn missing_database_entry_is_rejected() -> TestResult {
    let manifest = manifest_json(db::latest_migration_version(), b"db")?;
    let archive = archive_from(&[("manifest.json", &manifest)])?;

    let error = validate_restore_archive(&archive, db::latest_migration_version()).unwrap_err();
    assert!(matches!(error, RestoreError::MissingEntry(_)));
    Ok(())
}
