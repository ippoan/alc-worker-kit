# alc-worker-kit

分割 worker (Cloudflare Workers、workers-rs + tokio-postgres) が共通で使う部品。いま在るのは DB の部品の crate
`alc-worker-db` (`crates/alc-worker-db`) だけ (Refs ippoan/rust-alc-api#723)。

## 何の crate か

Hyperdrive 経由では、**名前付き prepared statement (`tx.execute`・`tx.query` など) を使うと接続が切れる** (PoC の実測)。
通るのは型付きの名前なしの文 (`query_typed` 系) だけ。これを呼び手の注意に任せると再発するので、
**`tokio_postgres::Client` と `Transaction` を呼び手に渡さず、型付きの文だけを出す型で包む。**
あわせて、テナント分離 (transaction 単位の RLS) の頭の文と、戻り値を owned に限る印を持つ。

## 公開する形

| 名前 | 形 |
|---|---|
| `PgClient` | `tokio_postgres::Client` を private に持つ。口は `PgClient::new(Client)`・`current_user(&self) -> Result<Option<String>, tokio_postgres::Error>` (固定の文 `SELECT current_user` を `simple_query` で 1 回)・`tenant_tx` の 3 つだけ |
| `PgClient::tenant_tx` | `async fn tenant_tx<T: TxOutput, F>(&mut self, tenant_id: Uuid, f: F) -> Result<T, tokio_postgres::Error>`、`F: for<'t> FnOnce(&'t TenantTx<'t>) -> BoxFuture<'t, Result<T, tokio_postgres::Error>> + Send`。順は固定: `BEGIN` → `SET_TENANT` → `f` → `COMMIT`。`f` が `Err` なら COMMIT せずに返す (drop = ROLLBACK) |
| `SET_TENANT` | `SELECT set_config('app.current_tenant_id', $1, true), set_config('search_path', 'alc_api', true)` (第 3 引数 `true` = transaction スコープ) |
| `TenantTx<'t>` | `Transaction<'t>` を private に持つ。口は `query_typed`・`query_typed_one`・`query_typed_opt`・`execute_typed` (行数 `u64`) の 4 つだけ。署名は tokio-postgres の同名のメソッドと同じ (`params: &[(&(dyn ToSql + Sync), Type)]`) |
| `TxOutput` | `pub trait TxOutput: Send + 'static {}`。transaction の外へ持ち出してよい値の印。kit が付けているのは `()`・`bool`・`i16`・`i32`・`i64`・`u64`・`String`・`uuid::Uuid`・(feature `chrono`) `chrono::DateTime<chrono::Utc>`・`Option<T>`・`Vec<T>`・2〜4 個のタプル。各 worker は**自分の crate の型**に impl する。`tokio_postgres::Row`・`Statement`・`RowStream` には付かない (外部の型なので利用側からも付けられない) |
| `kind(&tokio_postgres::Error) -> String` | エラーを、識別子を含まない固定の語に落とす。DB のエラーは SQLSTATE、ほかは固定の label (`io`・`closed`・`connect` など 12 種。当たらなければ `closed` / `other`)、原因が `std::io::Error` なら `:<ErrorKind の名前>` を足す。`Display` の文・DB の message は返さない |
| `hyperdrive::connect` (wasm32 専用) | `async fn connect(env: &worker::Env, binding: &str) -> Result<Option<PgClient>, ConnectError>`。binding が undefined = `Ok(None)` / 在るのに使えない = `Err` / 繋がった = `Ok(Some)`。接続文字列の `Config` は書き換えない |
| `hyperdrive::ConnectError` (wasm32 専用) | binding 名・段の label (`binding`・`config_parse`・`socket`・`handshake`)・`kind` だけを持つ。`Display` は `hyperdrive <binding>: <段>` か `hyperdrive <binding>: handshake: <kind>` |

`tokio_postgres::types::Type` と `ToSql` は、利用側が `tokio_postgres` から直接 import する (kit から re-export しない)。

## 使い方 (最小の例)

```rust
use alc_worker_db::PgClient;
use tokio_postgres::types::Type;
use uuid::Uuid;

// wasm32 の worker では: let Some(mut pg) = alc_worker_db::hyperdrive::connect(&env, "MY_HYPERDRIVE").await? else { … };
async fn rename(pg: &mut PgClient, tenant_id: Uuid, id: Uuid, name: String) -> Result<u64, tokio_postgres::Error> {
    pg.tenant_tx(tenant_id, move |tx| {
        Box::pin(async move {
            tx.execute_typed(
                "UPDATE things SET name = $2 WHERE id = $1",
                &[(&id, Type::UUID), (&name, Type::TEXT)],
            )
            .await
        })
    })
    .await
}
```

`Type::UUID`・`Type::TIMESTAMPTZ` を引数に渡すには、利用側の `tokio-postgres` に feature `with-uuid-1`・`with-chrono-0_4` が要る。
`Row` から値を取り出すのは閉包の中で済ませ、owned な値 (`TxOutput`) だけを返す。

## 足さないもの

**名前付きの文を呼ぶ口・任意の SQL を流す口・テナントを設定しない transaction を作る口を足さない。**
`PgClient` に `Deref`・`AsRef`・`GenericClient`・`into_inner`・`Clone` を、`TenantTx` に `query`・`execute`・`prepare`・
`simple_query`・`batch_execute`・`commit`・`client()` を足さない。crate の doc の `compile_fail` の doctest が、
`Row` を返せない・`TenantTx` に `query` / `execute` / `batch_execute` が無い・`PgClient` から生の `Client` を取れない、を固定している。

## 型で塞げないもの

- 型付きの 1 文として `COMMIT` や `set_config(.., false)` (session スコープ) を流すことは止められない。
  **SQL の中身は各 worker の定数とレビューで見る。**
- **閉包の Future は `Send` を要求する** (axum の handler が要求するため)。R2 など JS の値の await は transaction の中に挟めない —
  短い transaction を 2 回に分ける (transaction を長く握らないことにもなる)。
- Hyperdrive → DB の TLS と証明書の検証は Hyperdrive の設定側で、この crate からは検査できない。

## 引き方

git 依存の rev 固定。**書くのは利用側の直下の `Cargo.toml` の `[workspace.dependencies]` の 1 か所だけ** (tag は打たない):

```toml
[workspace.dependencies]
alc-worker-db = { git = "https://github.com/ippoan/alc-worker-kit", rev = "<main の commit>", features = ["chrono"] }
```

worker と route の crate は `alc-worker-db = { workspace = true }` で継承する。rev を上げたら `cargo update -p alc-worker-db` で
`Cargo.lock` を一緒に更新し、`cargo tree -i tokio-postgres --target wasm32-unknown-unknown` と
`cargo tree -i worker --target wasm32-unknown-unknown` で版が 1 つであることを確かめる。

## 検査の回し方

toolchain は `rust-toolchain.toml` (1.92.0 + `wasm32-unknown-unknown`)。CI (`.github/workflows/ci.yml`) と同じ順:

```bash
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo clippy --locked --target wasm32-unknown-unknown --all-features -- -D warnings
cargo build --locked --target wasm32-unknown-unknown --all-features
# 実 DB のテスト + 行カバレッジ 100% の gate (下の使い捨ての DB を立ててから)
KIT_TEST_ADMIN_DATABASE_URL=postgresql://postgres:<その場の文字列>@127.0.0.1:<port>/postgres \
  cargo llvm-cov --locked --all-features --fail-under-lines 100
cargo test --locked --doc --all-features   # compile_fail の doctest
```

実 DB のテストは 2 か所に在り、どちらも postgres 16 の**使い捨ての DB** に向ける (準備と接続は `crates/alc-worker-db/tests/support/mod.rs` の 1 つを共有):

- `crates/alc-worker-db/tests/tenant_tx_db.rs` — 公開の口だけを使う検査
- `crates/alc-worker-db/src/tx.rs` の `#[cfg(test)] mod tests` — 「COMMIT / ROLLBACK の後、**その接続に**設定が残っていない」。
  `PgClient` の中の `Client` を同じ接続のまま読む必要が在るので crate の中に置く (このために口を広げない)

```bash
docker run -d --rm --name <自分の名前> -e POSTGRES_PASSWORD=<その場の文字列> -p 127.0.0.1::5432 postgres:16
docker port <自分の名前>          # 割り当てられたポートを読む
docker rm -f <自分の名前>         # 終わったら自分のぶんだけ消す
```

- **`KIT_TEST_ADMIN_DATABASE_URL` (準備用の superuser) が未設定なら失敗する** (skip にしない)。
- テストが自分で schema `alc_api`・表 `kit_rls_probe` (RLS の式は ippoan/alc-migrations の `vein_templates` と同じ、`FORCE` なし)・
  ロール `kit_test_rt` (`LOGIN`・`NOSUPERUSER`・`NOBYPASSRLS`・表の所有者ではない) を作り、**本体のテストはそのロールで繋ぐ**
  (接続のたびに `pg_roles` を読み、superuser / BYPASSRLS なら panic)。ロールのパスワードはテストの中の固定の文字列で、使い捨ての DB 専用。
- 確かめていること: transaction の中の設定と search_path / COMMIT・ROLLBACK の後、その接続に設定が残らない / `Err` で書き込みが残らない /
  1 つの `PgClient` で 200 回続けて開ける / 1 transaction の中の複数の文と分岐 /
  RLS だけで他テナントが止まる (WHERE なしの読み取り・他テナントの `tenant_id` での INSERT は 42501) /
  テナントを設定しない transaction は読めない (テストが張った素の `Client` で) / `execute_typed` の行数と `UUID`・`TIMESTAMPTZ`・`TEXT`・`TEXT_ARRAY`・`INT4` の引数 /
  並列 (接続 8 本・テナント 2 つ・各 50 tx) / `current_user()` / `kind` (SQLSTATE・切れた接続)。
- CI は走ったテストの本数を、lib (6 本) と `tests/` (15 本) の両方について固定で見る (テストを足したら `ci.yml` の本数も上げる)。

## 限界

- llvm-cov の行 100% は、一度も呼ばれない generic を数えない。
- `hyperdrive` module は native では compile されない。CI が確かめるのは wasm32 の clippy とビルドまでで、
  動作は実験用の worker と本番で確かめる。
- エラーコード付きの `compile_fail` (`compile_fail,E0277` など) を照合するのは nightly だけ。stable は「落ちること」だけを見る
  (通る版を隣に置いて、違いを 1 行にしてある)。
