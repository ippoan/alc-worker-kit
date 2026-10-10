---
name: alc-worker-kit-map
generated-from: alc-worker-kit:37573b48bc0a33becea320ef6b73a2ab44e96075
paths: [crates/, .github/workflows/]
description: alc-worker-kit (分割 worker = Cloudflare Workers、workers-rs + tokio-postgres が共通で使う部品。DB の部品の crate `alc-worker-db` と、テナントのヘッダの layer・型・エラーの crate `alc-core-wasm` と、暗号の crate `alc-worker-crypto`) の構造ナビゲーション。どこに何があるか / 公開する形 / テナントの transaction の張り方 / Hyperdrive への接続 / 実 DB のテストと CI / 利用側からの引き方を 1 枚にまとめる。トリガー:「alc-worker-kit」「alc-worker-db」「PgClient」「TenantTx」「tenant_tx」「TxOutput」「SET_TENANT」「query_typed」「execute_typed」「hyperdrive::connect」「ConnectError」「KIT_TEST_ADMIN_DATABASE_URL」「kit_rls_probe」「名前付き prepared statement」「alc-core-wasm」「require_tenant_header」「TenantId」「AuthUser」「DbError」「api_error」「DeviceDevSlot」「alc-worker-crypto」「decrypt_secret」「encrypt_secret」「SSO_ENCRYPTION_KEY」「sign_rs256」等。
---

# alc-worker-kit-map — alc-worker-kit 構造ナビゲーション

分割 worker が共通で使う部品の workspace (Refs ippoan/rust-alc-api#723)。直下の `Cargo.toml` が workspace の root
(package なし、`members = ["crates/alc-core-wasm", "crates/alc-worker-crypto", "crates/alc-worker-db"]`、`Cargo.lock` は直下の 1 つ)。toolchain は `rust-toolchain.toml`
(1.92.0 + `wasm32-unknown-unknown`)。deploy・tag は無い (利用側は git 依存の rev 固定)。

| 場所 | 中身 |
|---|---|
| `crates/alc-worker-db/src/lib.rs` | 公開の入口 (`pub use`) と crate の doc。**`compile_fail` の doctest** (`Row` / `Vec<Row>` を `tenant_tx` から返せない・`TenantTx` に `query` / `execute` / `batch_execute` が無い・`PgClient` から生の `Client` を取れない) が、通る版 (`no_run`) と 1 行違いで並ぶ。crate 先頭で `unwrap_used`・`expect_used`・`panic`・`indexing_slicing` を deny (テストは除く) |
| `crates/alc-worker-db/src/tx.rs` | `SET_TENANT` (transaction の頭の文。`set_config('app.current_tenant_id', $1, true)` と search_path `alc_api`) / `PgClient` (`Client` を private に持つ。口は `new`・`current_user` = 固定の文を `simple_query` で 1 回・`tenant_tx` の 3 つ) / `tenant_tx` (`BEGIN` → `SET_TENANT` を `query_typed` で → `f` → `COMMIT`。`f` が `Err` なら drop = ROLLBACK。閉包の Future は `Send`) / `TenantTx<'t>` (`Transaction` を private に持つ。口は `query_typed`・`query_typed_one`・`query_typed_opt`・`execute_typed` の 4 つ) / `TxOutput` (transaction の外へ出してよい owned な値の印。kit の impl は `()`・`bool`・`i16`・`i32`・`i64`・`u64`・`String`・`Uuid`・feature `chrono` の `DateTime<Utc>`・`Option`・`Vec`・2〜4 個のタプル)。**`#[cfg(test)] mod tests` (実 DB、2 本)**: COMMIT / ROLLBACK の後、その接続に `app.current_tenant_id` と search_path が残っていないことを、private な `Client` の `simple_query` で読む (`tests/` からは触れないのでここに在る。口は広げない) |
| `crates/alc-worker-db/src/kind.rs` | `kind(&tokio_postgres::Error) -> String`: DB のエラーは SQLSTATE、ほかは表 `PG_KINDS` (`Display` の先頭 12 種 → 固定の label)、当たらなければ `closed` / `other`、原因が `io::Error` なら `:<ErrorKind>`。判定の本体は private の純粋な関数 `label` (unit test 4 本がここ) |
| `crates/alc-worker-db/src/hyperdrive.rs` | **wasm32 専用** (`#[cfg(target_arch = "wasm32")]`。native では compile されない)。`connect(env, binding) -> Result<Option<PgClient>, ConnectError>`: `Reflect::get` が undefined = `Ok(None)` → `env.hyperdrive` → 接続文字列を `Config` に (書き換えない) → StartTls の `Socket` → `connect_raw(socket, PassthroughTls)` → connection を `spawn_local` (Err は `kind` の label だけを `console_error!`)。`ConnectError` は binding 名・段 (`binding`・`config_parse`・`socket`・`handshake`)・`kind` だけ |
| `crates/alc-worker-db/tests/tenant_tx_db.rs` | 公開の口だけを使う実 DB のテスト (native、postgres 17、15 本): 設定と search_path・`Err` で書き込みが残らない・RLS だけで他テナントが止まる・引数の型 (UUID / TIMESTAMPTZ / TEXT / TEXT_ARRAY / INT4) と行数・200 回連続・1 tx の中の分岐・並列・ロール・`kind`。テナント未設定の transaction の検査だけ、テストが張った素の `Client` を使う |
| `crates/alc-worker-db/tests/support/mod.rs` | 実 DB のテストの準備と接続 (上のテストと `src/tx.rs` の中のテストが共有。lib 側は `src/lib.rs` の `#[cfg(test)] #[path] mod test_support`)。接続先 `KIT_TEST_ADMIN_DATABASE_URL` (準備用の superuser) が**無ければ失敗**。準備 (`PREPARE`。advisory lock で直列) が schema `alc_api`・表 `kit_rls_probe` (RLS は alc-migrations の `vein_templates` と同じ式、FORCE なし)・ロール `kit_test_rt` を作り、本体は `rt_raw_connect` = `kit_test_rt` で繋ぐ (毎回 `pg_roles` を読み superuser / BYPASSRLS なら panic) |
| `crates/alc-core-wasm/src/` | **rust-alc-api d28944e から写した** (Refs ippoan/rust-alc-api#736。`BUILD.bazel` は写していない。差は `device_dev.rs` の tests module の `assert!` 2 か所を 1 行にしただけ = 行カバレッジ 100% のため)。`tenant_header.rs` = `require_tenant_header` (axum の middleware。`X-Tenant-ID` が無い / UUID でない → 401、`TenantId` を extensions に入れ、`X-User-ID`・`X-User-Email`・`X-User-Role` が揃えば `AuthUser` も、`DeviceDevSlot` が在れば dev / 運行管理者の印を書く) / `types.rs` = `TenantId`・`AuthUser` / `db_error.rs` = `DbError` (feature `sqlx` で `From<sqlx::Error>`、`23505` → `Conflict`) / `api_error.rs` = `(StatusCode, Json)` を返す `bad_request`・`unprocessable`・`not_found`・`upstream_error`・`internal_error` (`STAGING_MODE=true` のときだけ `detail`)・`internal_error_msg` / `device_dev.rs` = `X-Device-Dev`・`X-Device-Role` を読む関数と `DeviceDevSlot`。unit test 19 本 |
| `crates/alc-core-wasm/tests/` | kit で足したテスト (src を変えないため `tests/` に置く): `api_error.rs` (6 本。各関数の status と body を丸ごと比べる・`STAGING_MODE` は 1 本の中で書き換える) / `db_error_sqlx.rs` (4 本、`#![cfg(feature = "sqlx")]`。偽の `DatabaseError` で SQLSTATE の分岐を見る) |
| `crates/alc-worker-crypto/src/` | Refs ippoan/rust-alc-api#747。alc-lineworks-worker の `secret.rs`・`jwt.rs` (9d76fd5) から移した。`secret.rs` = `decrypt_secret`・`decrypt_pem_secret`・`encrypt_secret` (乱数の nonce)・`encrypt_secret_with_nonce`・`normalize_pem_newlines`・`DecryptError`・`EncryptError` (AES-256-GCM、鍵 = SHA-256(`SSO_ENCRYPTION_KEY`)、rust-alc-api の ring と同じ形式。tests に ring の実装の写しが在り相互に読めることを見る) / `jwt.rs` = `sign_rs256<C: Serialize>(pem, kid, claims)`・`JwtError` (PKCS#1 / PKCS#8)。unit test 13 本。crate 先頭で `unwrap_used` 等を deny (テストは除く) |
| `.github/workflows/ci.yml` | job `kit` (postgres 17 の service container): fmt → clippy (native、`--all-features --all-targets`) → clippy (wasm32) → wasm32 のビルド → `cargo llvm-cov --all-features --fail-under-lines 100` (テストの実行を兼ねる) → **走った本数の固定** (test binary ごと: `alc_core_wasm` lib 19・`api_error` 6・`db_error_sqlx` 4・`alc_worker_crypto` lib 13・`alc_worker_db` lib 6・`tenant_tx_db` 15、0 ignored。`Running` の行の対象と `deps/<名前>-` の両方で区別し、色のコードを落としてから照合。テストを足したらここも上げる) → `cargo test --doc` (`compile_fail`)。`auto-merge` は ippoan/ci-workflows の reusable (`checks: read` が要る) |

## 依存

| 依存 | どこで |
|---|---|
| `tokio-postgres` 0.7 (`default-features = false`。wasm32 では feature `js`)・`uuid`・`futures-util` (BoxFuture)・任意 feature `chrono` | 全 target |
| `worker` 0.8 (feature `tokio-postgres`)・`wasm-bindgen`・`wasm-bindgen-futures` | wasm32 だけ (`hyperdrive` module) |
| `tokio`・runtime 付きの `tokio-postgres`・`chrono`・`uuid` の `v4` | native の dev-dependencies (実 DB のテスト) |
| `aes-gcm` 0.10・`base64` 0.22・`getrandom` 0.2 (wasm32 は `js`)・`rsa` 0.9・`serde`・`serde_json`・`sha2` 0.10 / dev: `rand` 0.8・`ring` 0.17 | `alc-worker-crypto` |
| `axum` 0.8 (`json`)・`serde`・`serde_json`・`thiserror` 2・`tracing`・`uuid` (wasm32 は `js`)・任意 feature `sqlx` 0.8 (default-features なし) | `alc-core-wasm` |

`alc-worker-db` は `alc-core-wasm` に依存しない (同じ workspace に同居するだけ)。利用側は直下の `[workspace.dependencies]` の 1 か所に git 依存 (rev 固定、2 crate で同じ rev) を書く (README の「引き方」)。
利用側の引き先を rust-alc-api から kit へ切り替えるのは後の PR — 1 つの依存の木で `alc-core-wasm` の出どころを混ぜない。
