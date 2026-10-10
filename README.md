# alc-worker-kit

分割 worker (Cloudflare Workers、workers-rs + tokio-postgres) が共通で使う部品。crate は 3 つ:

| crate | 中身 |
|---|---|
| `alc-worker-db` (`crates/alc-worker-db`) | DB の部品 (テナントの transaction を型で包む・Hyperdrive への接続)。Refs ippoan/rust-alc-api#723。下の「何の crate か」以降はこの crate の話 |
| `alc-worker-crypto` (`crates/alc-worker-crypto`) | 暗号の部品。`secret` = DB に置く秘密値の AES-256-GCM (`encrypt_secret`・`decrypt_secret`・`decrypt_pem_secret`・`normalize_pem_newlines`)、`jwt` = `sign_rs256`。下の「alc-worker-crypto」 |
| `alc-core-wasm` (`crates/alc-core-wasm`) | テナントのヘッダの layer (`require_tenant_header`)・`TenantId`・`AuthUser`・`DbError`・`api_error`・`device_dev`。下の「alc-core-wasm」 |

`alc-worker-db` は `alc-core-wasm` に依存しない (同じ workspace に同居するだけ)。`alc-worker-crypto` もほかの 2 つに依存しない。

## alc-worker-crypto

Refs ippoan/rust-alc-api#747。ippoan/alc-lineworks-worker の `crates/lineworks/src/secret.rs`・`jwt.rs` (9d76fd5) を移し、署名を汎用にした
(LINE WORKS / LINE の両方の worker が同じものを使う。3 つ目の写しを作らない)。依存の版は alc-lineworks-worker と同じ。pure Rust (RustCrypto) で native と wasm32 の両方でビルドする。

| 名前 | 形 |
|---|---|
| `secret::decrypt_secret(ciphertext_b64, key_material) -> Result<String, DecryptError>` | 形式は `base64(nonce[12] + ciphertext + tag[16])`、AES-256-GCM、鍵 = SHA-256(`key_material`)、AAD なし (rust-alc-api の `alc_core::auth_lineworks` と同じ) |
| `secret::decrypt_pem_secret` | `decrypt_secret` → `normalize_pem_newlines` (rust-alc-api の同名の関数と同じ) |
| `secret::encrypt_secret(plaintext, key_material) -> Result<String, EncryptError>` | nonce は乱数 (`getrandom`。wasm32 は `crypto.getRandomValues`) |
| `secret::encrypt_secret_with_nonce` | nonce を呼び手が渡す (テスト用。同じ鍵で nonce を使い回さない) |
| `secret::normalize_pem_newlines` | 実の改行が無く `\n` (2 文字) が在るときだけ改行に直す |
| `jwt::sign_rs256<C: Serialize>(private_key_pem, kid: Option<&str>, claims: &C) -> Result<String, JwtError>` | header は `{"alg":"RS256","typ":"JWT"}`、`kid` を渡せば `"kid"` を足す (LINE の channel access token v2.1)。PEM は PKCS#1 / PKCS#8 の両方、前後の空白と `\n` (2 文字) も受ける。claim は呼び手の型 (LINE WORKS / LINE 固有の claim を組む関数は各 worker に置く) |

エラー (`DecryptError` = `base64`・`too_short`・`aead`・`utf8` / `EncryptError` = `rng` / `JwtError` = `key`・`claims`) は段の名前だけで、暗号文・鍵・平文・claim を持たない。
テストは lib 13 本。rust-alc-api の ring の実装をテストに写し、ring で作った暗号文をここで読める・ここで作った暗号文を ring で読める・固定の値の暗号文が ring と 1 byte も違わない、を確かめる。

## alc-core-wasm

Cloud Run (ippoan/rust-alc-api) と分割 worker (alc-dtako-worker・alc-vein-worker) が共有する部分を、
**rust-alc-api の commit `d28944e` (`crates/alc-core-wasm`) から写した** (Refs ippoan/rust-alc-api#736)。
src の差は、tests module の 2 か所 (`device_dev.rs` の、失敗時メッセージ引数が別の行に在った `assert!`) だけを 1 行の `assert!` に書き換えた — 行カバレッジ 100% の gate のため。本体のコードは写したまま (`BUILD.bazel` は写していない。`Cargo.toml` の差は `repository` だけ)。
**利用側 (alc-dtako-worker・alc-vein-worker・rust-alc-api) の引き先を kit に切り替えるのは後の PR。** 切り替えが済むまでは
rust-alc-api の側も残っているので、**利用側の 1 つの依存の木で出どころを混ぜない** (`TenantId` が別の型になり、
コンパイルは通るのに全リクエストが 500 になる)。確かめ方: `cargo tree -i alc-core-wasm --target wasm32-unknown-unknown` で出どころが 1 つ。

- native と wasm32 の両方でビルドする。sqlx は任意 feature `sqlx` (`From<sqlx::Error> for DbError`) だけ。
- テストは src の unit test (19 本) と、kit で足した `crates/alc-core-wasm/tests/` (`api_error.rs` = 各関数の status と body・
  `STAGING_MODE` で `detail` を出す / 隠す、`db_error_sqlx.rs` = unique_violation の SQLSTATE `23505` だけが `Conflict` になる)。

## 何の crate か (alc-worker-db)

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
alc-core-wasm = { git = "https://github.com/ippoan/alc-worker-kit", rev = "<同じ commit>" }
alc-worker-crypto = { git = "https://github.com/ippoan/alc-worker-kit", rev = "<同じ commit>" }
```

worker と route の crate は `alc-worker-db = { workspace = true }` (`alc-core-wasm`・`alc-worker-crypto` も同じ) で継承する。kit から引く crate の rev は揃える (使わない crate は書かない)。
rev を上げたら `cargo update -p alc-worker-db -p alc-core-wasm` (引いている crate を全部) で `Cargo.lock` を一緒に更新し、`cargo tree -i tokio-postgres --target wasm32-unknown-unknown` と
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

実 DB のテストは 2 か所に在り、どちらも postgres 17 の**使い捨ての DB** に向ける。版は本番 (PostgreSQL 17 系) に合わせる (準備と接続は `crates/alc-worker-db/tests/support/mod.rs` の 1 つを共有):

- `crates/alc-worker-db/tests/tenant_tx_db.rs` — 公開の口だけを使う検査
- `crates/alc-worker-db/src/tx.rs` の `#[cfg(test)] mod tests` — 「COMMIT / ROLLBACK の後、**その接続に**設定が残っていない」。
  `PgClient` の中の `Client` を同じ接続のまま読む必要が在るので crate の中に置く (このために口を広げない)

```bash
docker run -d --rm --name <自分の名前> -e POSTGRES_PASSWORD=<その場の文字列> -p 127.0.0.1::5432 postgres:17
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
- CI は走ったテストの本数を test binary ごとに固定で見る (テストを足したら `ci.yml` の本数も上げる):
  `alc-worker-db` は lib 6 本・`tests/tenant_tx_db.rs` 15 本、`alc-core-wasm` は lib 19 本・`tests/api_error.rs` 6 本・`tests/db_error_sqlx.rs` 4 本、`alc-worker-crypto` は lib 13 本。
  lib.rs を持つ crate が 3 つ在るので、`Running` の行の対象と test binary の名前 (`deps/alc_worker_db-` 等) の両方で区別し、色のコードは落としてから照合する。

## 限界

- llvm-cov の行 100% は、一度も呼ばれない generic を数えない。
- 同じ関数が lib の unit test の binary と `tests/` の binary に別々に入ると、llvm-cov の行の集計は**それぞれの binary の側で**行を数える
  (`report --text` では通っている行が、要約では未到達に数えられることがある)。`tests/db_error_sqlx.rs` が `RowNotFound` も見ているのはこのため。
- `hyperdrive` module は native では compile されない。CI が確かめるのは wasm32 の clippy とビルドまでで、
  動作は実験用の worker と本番で確かめる。
- エラーコード付きの `compile_fail` (`compile_fail,E0277` など) を照合するのは nightly だけ。stable は「落ちること」だけを見る
  (通る版を隣に置いて、違いを 1 行にしてある)。
