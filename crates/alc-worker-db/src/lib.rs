//! 分割 worker (Cloudflare Workers、workers-rs + tokio-postgres) が共通で使う DB の部品
//! (Refs ippoan/rust-alc-api#723)。
//!
//! Hyperdrive 経由では、**名前付き prepared statement (`tx.execute`・`tx.query` など) を使うと接続が切れる**
//! (PoC の実測)。通るのは型付きの名前なしの文 (`query_typed` 系) だけ。これを呼び手の注意に任せず、
//! `tokio_postgres::Client` と `Transaction` を呼び手に渡さない型で塞ぐ:
//!
//! - [`PgClient`] — `Client` の包み。口は [`PgClient::new`]・[`PgClient::current_user`]・[`PgClient::tenant_tx`] だけ
//! - [`TenantTx`] — テナントを設定済みの transaction。口は `query_typed`・`query_typed_one`・
//!   `query_typed_opt`・`execute_typed` の 4 つだけ
//! - [`TxOutput`] — transaction の外へ持ち出してよい値の印 (owned な型だけ)
//! - [`SET_TENANT`] — transaction の頭で打つ文 (テナントと search_path を transaction スコープで設定する)
//! - [`kind`] — `tokio_postgres::Error` を、識別子を含まない固定の語に落とす
//! - `hyperdrive::connect` (wasm32 専用) — Hyperdrive の binding から繋いで [`PgClient`] を返す
//!
//! `tokio_postgres::types::Type` と `ToSql` は、利用側が `tokio_postgres` から直接 import する。
//!
//! # 使い方
//!
//! ```no_run
//! use alc_worker_db::PgClient;
//! use tokio_postgres::types::Type;
//!
//! async fn rename(
//!     pg: &mut PgClient,
//!     tenant_id: uuid::Uuid,
//!     note: String,
//! ) -> Result<u64, tokio_postgres::Error> {
//!     pg.tenant_tx(tenant_id, |tx| {
//!         Box::pin(async move {
//!             tx.execute_typed("UPDATE things SET note = $1", &[(&note, Type::TEXT)])
//!                 .await
//!         })
//!     })
//!     .await
//! }
//! ```
//!
//! # 型で塞いでいること (`compile_fail` の doctest)
//!
//! どの組も、通る版 (`no_run`) と通らない版 (`compile_fail`) の違いは **1 行だけ**
//! (`// ←` の行。ほかの理由で落ちていないことを、通る版が示す)。
//!
//! ## `Row` を transaction の外へ持ち出せない
//!
//! 通る版 (`Row` から owned な値を取り出して返す):
//!
//! ```no_run
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     let _out = pg
//!         .tenant_tx(tenant_id, |tx| {
//!             Box::pin(async move {
//!                 let row = tx.query_typed_one("SELECT 1::int8", &[]).await?;
//!                 Ok(row.get::<_, i64>(0)) // ←
//!             })
//!         })
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! `Row` をそのまま返すと通らない (`Row: TxOutput` が成り立たない):
//!
//! ```compile_fail,E0277
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     let _out = pg
//!         .tenant_tx(tenant_id, |tx| {
//!             Box::pin(async move {
//!                 let row = tx.query_typed_one("SELECT 1::int8", &[]).await?;
//!                 Ok(row) // ←
//!             })
//!         })
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! 通る版 (`Vec<i64>` にしてから返す):
//!
//! ```no_run
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     let _out = pg
//!         .tenant_tx(tenant_id, |tx| {
//!             Box::pin(async move {
//!                 let rows = tx.query_typed("SELECT 1::int8", &[]).await?;
//!                 Ok(rows.iter().map(|r| r.get(0)).collect::<Vec<i64>>()) // ←
//!             })
//!         })
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! `Vec<Row>` をそのまま返すと通らない:
//!
//! ```compile_fail,E0277
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     let _out = pg
//!         .tenant_tx(tenant_id, |tx| {
//!             Box::pin(async move {
//!                 let rows = tx.query_typed("SELECT 1::int8", &[]).await?;
//!                 Ok(rows) // ←
//!             })
//!         })
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! ## `TenantTx` に名前付きの文・複数の文を流す口が無い
//!
//! 通る版 (型付きの名前なしの文):
//!
//! ```no_run
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     pg.tenant_tx(tenant_id, |tx| {
//!         Box::pin(async move {
//!             tx.execute_typed("SELECT 1", &[]).await?; // ←
//!             Ok(())
//!         })
//!     })
//!     .await
//! }
//! ```
//!
//! `query` (名前付き prepared statement) は無い:
//!
//! ```compile_fail,E0599
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     pg.tenant_tx(tenant_id, |tx| {
//!         Box::pin(async move {
//!             tx.query("SELECT 1", &[]).await?; // ←
//!             Ok(())
//!         })
//!     })
//!     .await
//! }
//! ```
//!
//! `execute` (名前付き prepared statement) は無い:
//!
//! ```compile_fail,E0599
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     pg.tenant_tx(tenant_id, |tx| {
//!         Box::pin(async move {
//!             tx.execute("SELECT 1", &[]).await?; // ←
//!             Ok(())
//!         })
//!     })
//!     .await
//! }
//! ```
//!
//! `batch_execute` (複数の文。`COMMIT; BEGIN` を流せてしまう) は無い:
//!
//! ```compile_fail,E0599
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: &mut PgClient, tenant_id: uuid::Uuid) -> Result<(), tokio_postgres::Error> {
//!     pg.tenant_tx(tenant_id, |tx| {
//!         Box::pin(async move {
//!             tx.batch_execute("SELECT 1").await?; // ←
//!             Ok(())
//!         })
//!     })
//!     .await
//! }
//! ```
//!
//! ## `PgClient` から生の `Client` を取り出せない
//!
//! 通る版:
//!
//! ```no_run
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: PgClient) -> Result<(), tokio_postgres::Error> {
//!     let _user = pg.current_user().await?; // ←
//!     Ok(())
//! }
//! ```
//!
//! field は private:
//!
//! ```compile_fail,E0616
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: PgClient) -> Result<(), tokio_postgres::Error> {
//!     let _client: &tokio_postgres::Client = &pg.client; // ←
//!     Ok(())
//! }
//! ```
//!
//! `into_inner` は無い:
//!
//! ```compile_fail,E0599
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: PgClient) -> Result<(), tokio_postgres::Error> {
//!     let _client: tokio_postgres::Client = pg.into_inner(); // ←
//!     Ok(())
//! }
//! ```
//!
//! `Deref` は無い:
//!
//! ```compile_fail,E0614
//! use alc_worker_db::PgClient;
//!
//! async fn f(pg: PgClient) -> Result<(), tokio_postgres::Error> {
//!     let _client: &tokio_postgres::Client = &*pg; // ←
//!     Ok(())
//! }
//! ```

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

#[cfg(target_arch = "wasm32")]
pub mod hyperdrive;
mod kind;
mod tx;

pub use kind::kind;
pub use tx::{PgClient, TenantTx, TxOutput, SET_TENANT};
