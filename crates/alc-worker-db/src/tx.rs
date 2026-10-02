//! テナントの transaction と、その中で流せる文を型付きの名前なしの文に限る包み。

use futures_util::future::BoxFuture;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Error, Row, SimpleQueryMessage, Transaction};
use uuid::Uuid;

/// [`PgClient::tenant_tx`] が transaction の頭で打つ文。`set_config` の第 3 引数 `true` は
/// transaction スコープ (= `SET LOCAL`。COMMIT / ROLLBACK で消える)。search_path も同じ文で設定する
/// (プーラーが接続時の設定を上流へ渡す保証が無く、DB の既定に頼らないため)。
pub const SET_TENANT: &str = "SELECT set_config('app.current_tenant_id', $1, true), set_config('search_path', 'alc_api', true)";

/// `tokio_postgres::Client` の包み。**生の `Client` は取り出せない。** 口は [`PgClient::new`]・
/// [`PgClient::current_user`]・[`PgClient::tenant_tx`] の 3 つだけで、名前付き prepared statement も
/// 任意の SQL も、テナントを設定しない transaction も、ここからは流せない。
pub struct PgClient {
    client: Client,
}

impl PgClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// この接続の `current_user`。固定の文を `simple_query` (prepared statement を作らない) で 1 回流す。
    /// テナントを取らないので transaction は張らない。行が無ければ `Ok(None)`。
    pub async fn current_user(&self) -> Result<Option<String>, Error> {
        let messages = self.client.simple_query("SELECT current_user").await?;
        Ok(messages.iter().find_map(|m| match m {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        }))
    }

    /// テナントを設定した transaction を開き、`f` の結果を受け取ってから COMMIT する。
    ///
    /// 順は固定: `BEGIN` → [`SET_TENANT`] → `f` → `COMMIT`。`f` が `Err` を返したら COMMIT せずに返す
    /// (`Transaction` の drop = ROLLBACK)。`f` の戻り値は [`TxOutput`] (owned な型) に限るので、
    /// `Row` を transaction の外へ持ち出すコードはコンパイルが通らない。
    ///
    /// `f` の Future は `Send` を要求する。JS の値 (R2 など) の await は中に挟めない —
    /// 短い transaction を 2 回に分ける。
    pub async fn tenant_tx<T, F>(&mut self, tenant_id: Uuid, f: F) -> Result<T, Error>
    where
        T: TxOutput,
        F: for<'t> FnOnce(&'t TenantTx<'t>) -> BoxFuture<'t, Result<T, Error>> + Send,
    {
        let tx = self.client.transaction().await?;
        tx.query_typed(SET_TENANT, &[(&tenant_id.to_string(), Type::TEXT)])
            .await?;
        let tx = TenantTx { tx };
        let out = f(&tx).await?;
        tx.tx.commit().await?;
        Ok(out)
    }
}

/// テナントを設定済みの transaction。出すのは型付きの名前なしの文の 4 つだけ
/// (署名は `tokio_postgres::Transaction` の同名のメソッドと同じ)。
///
/// 名前付き prepared statement を作る口 (`query`・`execute`・`prepare`)、複数の文を流せる口
/// (`simple_query`・`batch_execute`)、`commit`、生の `Client` へ降りる口は無い。
pub struct TenantTx<'t> {
    tx: Transaction<'t>,
}

impl TenantTx<'_> {
    pub async fn query_typed(
        &self,
        sql: &str,
        params: &[(&(dyn ToSql + Sync), Type)],
    ) -> Result<Vec<Row>, Error> {
        self.tx.query_typed(sql, params).await
    }

    pub async fn query_typed_one(
        &self,
        sql: &str,
        params: &[(&(dyn ToSql + Sync), Type)],
    ) -> Result<Row, Error> {
        self.tx.query_typed_one(sql, params).await
    }

    pub async fn query_typed_opt(
        &self,
        sql: &str,
        params: &[(&(dyn ToSql + Sync), Type)],
    ) -> Result<Option<Row>, Error> {
        self.tx.query_typed_opt(sql, params).await
    }

    /// 影響した行数を返す
    pub async fn execute_typed(
        &self,
        sql: &str,
        params: &[(&(dyn ToSql + Sync), Type)],
    ) -> Result<u64, Error> {
        self.tx.execute_typed(sql, params).await
    }
}

/// transaction の外へ持ち出してよい値の印 (owned な型だけに付ける)。
///
/// `tokio_postgres::Row`・`Statement`・`RowStream` には付けない (この crate も、利用側も — 外部の型なので
/// 利用側からは orphan rule で付けられない)。各 worker は**自分の crate の型**に impl する。
pub trait TxOutput: Send + 'static {}

impl TxOutput for () {}
impl TxOutput for bool {}
impl TxOutput for i16 {}
impl TxOutput for i32 {}
impl TxOutput for i64 {}
impl TxOutput for u64 {}
impl TxOutput for String {}
impl TxOutput for Uuid {}
#[cfg(feature = "chrono")]
impl TxOutput for chrono::DateTime<chrono::Utc> {}
impl<T: TxOutput> TxOutput for Option<T> {}
impl<T: TxOutput> TxOutput for Vec<T> {}
impl<A: TxOutput, B: TxOutput> TxOutput for (A, B) {}
impl<A: TxOutput, B: TxOutput, C: TxOutput> TxOutput for (A, B, C) {}
impl<A: TxOutput, B: TxOutput, C: TxOutput, D: TxOutput> TxOutput for (A, B, C, D) {}
