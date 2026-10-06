/// DB 層の共通エラー。sqlx 非依存 (wasm では D1 / Hyperdrive 等の別実装から作る)。
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("{0}")]
    Other(String),
}

/// PostgreSQL の unique_violation。
#[cfg(feature = "sqlx")]
const PG_UNIQUE_VIOLATION: &str = "23505";

#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => DbError::NotFound,
            sqlx::Error::Database(db) if db.code().as_deref() == Some(PG_UNIQUE_VIOLATION) => {
                DbError::Conflict(db.message().to_string())
            }
            other => DbError::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        assert_eq!(DbError::NotFound.to_string(), "not found");
        assert_eq!(DbError::Conflict("x".into()).to_string(), "conflict: x");
        assert_eq!(DbError::Other("y".into()).to_string(), "y");
    }

    #[cfg(feature = "sqlx")]
    #[test]
    fn from_sqlx() {
        assert!(matches!(
            DbError::from(sqlx::Error::RowNotFound),
            DbError::NotFound
        ));
        assert!(matches!(
            DbError::from(sqlx::Error::PoolTimedOut),
            DbError::Other(_)
        ));
    }
}
