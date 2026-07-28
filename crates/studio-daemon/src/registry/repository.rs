use thiserror::Error;
use turso::{Database, params};

use super::{
    credential_store::RegistryCredentialId,
    identity::{RegistryIdentity, RegistryIdentityError},
};

const COLUMNS: &str =
    "id, registry_identity, username_label, created_at_ms, updated_at_ms, last_success_at_ms";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCredentialMetadata {
    pub id: RegistryCredentialId,
    pub registry: RegistryIdentity,
    pub username_label: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_success_at_ms: Option<i64>,
}

#[derive(Debug, Error)]
pub enum RegistryRepositoryError {
    #[error("registry credential database operation failed")]
    Database(#[from] turso::Error),
    #[error("registry credential metadata is invalid")]
    InvalidMetadata,
}

pub async fn insert(
    db: &Database,
    metadata: &RegistryCredentialMetadata,
) -> Result<(), RegistryRepositoryError> {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO registry_credentials (
            id, registry_identity, username_label, created_at_ms, updated_at_ms,
            last_success_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            metadata.id.as_str().to_owned(),
            metadata.registry.as_str().to_owned(),
            metadata.username_label.clone(),
            metadata.created_at_ms,
            metadata.updated_at_ms,
            metadata.last_success_at_ms
        ],
    )
    .await?;
    Ok(())
}

pub async fn list(
    db: &Database,
) -> Result<Vec<RegistryCredentialMetadata>, RegistryRepositoryError> {
    let conn = db.connect()?;
    let sql = format!(
        "SELECT {COLUMNS} FROM registry_credentials
         ORDER BY registry_identity ASC"
    );
    let mut rows = conn.query(&sql, ()).await?;
    let mut metadata = Vec::new();
    while let Some(row) = rows.next().await? {
        metadata.push(metadata_from_row(&row)?);
    }
    Ok(metadata)
}

pub async fn find_by_id(
    db: &Database,
    id: &RegistryCredentialId,
) -> Result<Option<RegistryCredentialMetadata>, RegistryRepositoryError> {
    let conn = db.connect()?;
    let sql = format!("SELECT {COLUMNS} FROM registry_credentials WHERE id = ?1");
    let mut rows = conn.query(&sql, params![id.as_str().to_owned()]).await?;
    rows.next()
        .await?
        .as_ref()
        .map(metadata_from_row)
        .transpose()
}

pub async fn find_by_registry(
    db: &Database,
    registry: &RegistryIdentity,
) -> Result<Option<RegistryCredentialMetadata>, RegistryRepositoryError> {
    let conn = db.connect()?;
    let sql = format!("SELECT {COLUMNS} FROM registry_credentials WHERE registry_identity = ?1");
    let mut rows = conn
        .query(&sql, params![registry.as_str().to_owned()])
        .await?;
    rows.next()
        .await?
        .as_ref()
        .map(metadata_from_row)
        .transpose()
}

pub async fn touch_success(
    db: &Database,
    id: &RegistryCredentialId,
    now_ms: i64,
) -> Result<(), RegistryRepositoryError> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE registry_credentials
         SET last_success_at_ms = ?1, updated_at_ms = ?1
         WHERE id = ?2",
        params![now_ms, id.as_str().to_owned()],
    )
    .await?;
    Ok(())
}

pub async fn delete(
    db: &Database,
    id: &RegistryCredentialId,
) -> Result<(), RegistryRepositoryError> {
    let conn = db.connect()?;
    conn.execute(
        "DELETE FROM registry_credentials WHERE id = ?1",
        params![id.as_str().to_owned()],
    )
    .await?;
    Ok(())
}

fn metadata_from_row(
    row: &turso::Row,
) -> Result<RegistryCredentialMetadata, RegistryRepositoryError> {
    let id = RegistryCredentialId::parse(row.get::<String>(0)?)
        .map_err(|_| RegistryRepositoryError::InvalidMetadata)?;
    let registry = RegistryIdentity::parse(&row.get::<String>(1)?)
        .map_err(|_: RegistryIdentityError| RegistryRepositoryError::InvalidMetadata)?;
    Ok(RegistryCredentialMetadata {
        id,
        registry,
        username_label: row.get(2)?,
        created_at_ms: row.get(3)?,
        updated_at_ms: row.get(4)?,
        last_success_at_ms: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fresh_db;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn metadata(registry: &str) -> TestResult<RegistryCredentialMetadata> {
        Ok(RegistryCredentialMetadata {
            id: RegistryCredentialId::new(),
            registry: RegistryIdentity::parse(registry)?,
            username_label: Some("studio-user".to_owned()),
            created_at_ms: 10,
            updated_at_ms: 10,
            last_success_at_ms: None,
        })
    }

    #[tokio::test]
    async fn metadata_roundtrips_without_secret_columns() -> TestResult {
        let db = fresh_db("registry-metadata-roundtrip").await?;
        let expected = metadata("registry.example")?;
        insert(&db, &expected).await?;

        assert_eq!(find_by_id(&db, &expected.id).await?, Some(expected.clone()));
        assert_eq!(
            find_by_registry(&db, &expected.registry).await?,
            Some(expected.clone())
        );
        assert_eq!(list(&db).await?, vec![expected.clone()]);

        let conn = db.connect()?;
        let mut rows = conn
            .query("PRAGMA table_info(registry_credentials)", ())
            .await?;
        let mut columns = Vec::new();
        while let Some(row) = rows.next().await? {
            columns.push(row.get::<String>(1)?);
        }
        for forbidden in ["password", "token", "secret", "auth"] {
            assert!(
                columns.iter().all(|column| !column.contains(forbidden)),
                "forbidden metadata column: {forbidden}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn one_metadata_row_is_allowed_per_normalized_registry() -> TestResult {
        let db = fresh_db("registry-metadata-unique").await?;
        insert(&db, &metadata("index.docker.io")?).await?;
        let duplicate = metadata("docker.io")?;
        assert!(matches!(
            insert(&db, &duplicate).await,
            Err(RegistryRepositoryError::Database(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn success_timestamp_and_delete_are_explicit() -> TestResult {
        let db = fresh_db("registry-metadata-lifecycle").await?;
        let expected = metadata("registry.example")?;
        insert(&db, &expected).await?;

        touch_success(&db, &expected.id, 42).await?;
        let updated = find_by_id(&db, &expected.id)
            .await?
            .ok_or("missing metadata")?;
        assert_eq!(updated.last_success_at_ms, Some(42));
        assert_eq!(updated.updated_at_ms, 42);

        delete(&db, &expected.id).await?;
        assert!(find_by_id(&db, &expected.id).await?.is_none());
        Ok(())
    }
}
