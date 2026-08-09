use serde::{Deserialize, Serialize};
use turso::{Database, params};

use super::now_ms;

#[derive(Debug, thiserror::Error)]
pub enum OnboardingError {
    #[error("runtime onboarding storage is unavailable: {0}")]
    Database(#[from] turso::Error),

    #[error("runtime onboarding singleton row is missing")]
    MissingSingleton,

    #[error("runtime onboarding contains an invalid state")]
    InvalidState,

    #[error("runtime onboarding contains an invalid choice")]
    InvalidChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingState {
    Pending,
    Completed,
    Dismissed,
}

impl OnboardingState {
    fn from_database(value: String) -> Result<Self, OnboardingError> {
        match value.as_str() {
            "pending" => Ok(Self::Pending),
            "completed" => Ok(Self::Completed),
            "dismissed" => Ok(Self::Dismissed),
            _ => Err(OnboardingError::InvalidState),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingChoice {
    BuiltIn,
    Existing,
}

impl OnboardingChoice {
    fn as_database(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Existing => "existing",
        }
    }

    fn from_database(value: String) -> Result<Self, OnboardingError> {
        match value.as_str() {
            "built_in" => Ok(Self::BuiltIn),
            "existing" => Ok(Self::Existing),
            _ => Err(OnboardingError::InvalidChoice),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeOnboarding {
    pub state: OnboardingState,
    pub choice: Option<OnboardingChoice>,
    pub completed_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

pub async fn read(db: &Database) -> Result<RuntimeOnboarding, OnboardingError> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT state, choice, completed_at_ms, updated_at_ms
             FROM runtime_onboarding WHERE singleton = 1",
            (),
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or(OnboardingError::MissingSingleton)?;
    let choice = row
        .get::<Option<String>>(1)?
        .map(OnboardingChoice::from_database)
        .transpose()?;
    Ok(RuntimeOnboarding {
        state: OnboardingState::from_database(row.get(0)?)?,
        choice,
        completed_at_ms: row.get(2)?,
        updated_at_ms: row.get(3)?,
    })
}

pub async fn complete(
    db: &Database,
    choice: OnboardingChoice,
) -> Result<RuntimeOnboarding, OnboardingError> {
    let now = now_ms();
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_onboarding
         SET state = 'completed', choice = ?1, completed_at_ms = ?2, updated_at_ms = ?2
         WHERE singleton = 1",
        params![choice.as_database(), now],
    )
    .await?;
    read(db).await
}

pub async fn dismiss(db: &Database) -> Result<RuntimeOnboarding, OnboardingError> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_onboarding
         SET state = 'dismissed', choice = NULL, completed_at_ms = NULL, updated_at_ms = ?1
         WHERE singleton = 1 AND state = 'pending'",
        params![now_ms()],
    )
    .await?;
    read(db).await
}

pub async fn reopen(db: &Database) -> Result<RuntimeOnboarding, OnboardingError> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_onboarding
         SET state = 'pending', choice = NULL, completed_at_ms = NULL, updated_at_ms = ?1
         WHERE singleton = 1 AND state = 'dismissed'",
        params![now_ms()],
    )
    .await?;
    read(db).await
}

#[cfg(test)]
mod tests {
    use super::{OnboardingChoice, OnboardingState, complete, dismiss, read, reopen};
    use crate::db;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn fresh_db() -> TestResult<(turso::Database, std::path::PathBuf)> {
        let path = std::env::temp_dir().join(format!(
            "studio-runtime-onboarding-test-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        Ok((db::open_database(path.clone()).await?, path))
    }

    #[tokio::test]
    async fn onboarding_starts_pending_and_transitions_without_arbitrary_state_writes() -> TestResult
    {
        let (db, path) = fresh_db().await?;

        assert_eq!(read(&db).await?.state, OnboardingState::Pending);
        assert_eq!(read(&db).await?.choice, None);

        dismiss(&db).await?;
        assert_eq!(read(&db).await?.state, OnboardingState::Dismissed);
        assert_eq!(read(&db).await?.choice, None);

        reopen(&db).await?;
        assert_eq!(read(&db).await?.state, OnboardingState::Pending);

        complete(&db, OnboardingChoice::Existing).await?;
        let completed = read(&db).await?;
        assert_eq!(completed.state, OnboardingState::Completed);
        assert_eq!(completed.choice, Some(OnboardingChoice::Existing));
        assert!(completed.completed_at_ms.is_some());

        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn onboarding_survives_reopening_the_database() -> TestResult {
        let (db, path) = fresh_db().await?;
        complete(&db, OnboardingChoice::BuiltIn).await?;
        drop(db);

        let reopened = db::open_database(path.clone()).await?;
        let state = read(&reopened).await?;
        assert_eq!(state.state, OnboardingState::Completed);
        assert_eq!(state.choice, Some(OnboardingChoice::BuiltIn));

        drop(reopened);
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
