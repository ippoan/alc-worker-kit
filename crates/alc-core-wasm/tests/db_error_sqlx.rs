//! feature `sqlx` の `From<sqlx::Error> for DbError` のうち、DB のエラー (`sqlx::Error::Database`)
//! の分岐を固定する。src の unit test (`db_error::tests::from_sqlx`) は `RowNotFound` と
//! `PoolTimedOut` だけを見ているので、SQLSTATE の分岐をここで足す (src は写したまま)。
#![cfg(feature = "sqlx")]

use std::{borrow::Cow, error::Error as StdError, fmt};

use alc_core_wasm::DbError;
use sqlx::error::{DatabaseError, ErrorKind};

/// SQLSTATE と message だけを持つ DB のエラー。
#[derive(Debug)]
struct FakeDbError {
    code: Option<&'static str>,
    message: &'static str,
}

impl fmt::Display for FakeDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl StdError for FakeDbError {}

impl DatabaseError for FakeDbError {
    fn message(&self) -> &str {
        self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        self.code.map(Cow::Borrowed)
    }

    fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

fn db_error(code: Option<&'static str>, message: &'static str) -> DbError {
    DbError::from(sqlx::Error::Database(Box::new(FakeDbError {
        code,
        message,
    })))
}

#[test]
fn unique_violation_becomes_conflict_with_db_message() {
    match db_error(Some("23505"), "duplicate key value") {
        DbError::Conflict(m) => assert_eq!(m, "duplicate key value"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn other_sqlstate_becomes_other() {
    // 23503 = foreign_key_violation。unique_violation 以外の DB のエラーは Conflict にしない
    match db_error(Some("23503"), "fk violation") {
        DbError::Other(m) => assert!(m.contains("fk violation"), "{m:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn db_error_without_sqlstate_becomes_other() {
    match db_error(None, "no code") {
        DbError::Other(m) => assert!(m.contains("no code"), "{m:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn row_not_found_becomes_not_found() {
    assert!(matches!(
        DbError::from(sqlx::Error::RowNotFound),
        DbError::NotFound
    ));
}
