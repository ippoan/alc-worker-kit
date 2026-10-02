---
name: alc-worker-kit-map
generated-from: alc-worker-kit:863da18d3d459eff4c8845a04da99e60dac6d84a
paths: [crates/, .github/workflows/]
description: alc-worker-kit (分割 worker = Cloudflare Workers、workers-rs + tokio-postgres が共通で使う部品。いまは DB の部品の crate `alc-worker-db` だけ) の構造ナビゲーション。どこに何があるか / 公開する形 / テナントの transaction の張り方 / Hyperdrive への接続 / 実 DB のテストと CI / 利用側からの引き方を 1 枚にまとめる。トリガー:「alc-worker-kit」「alc-worker-db」「PgClient」「TenantTx」「tenant_tx」「TxOutput」「SET_TENANT」「query_typed」「execute_typed」「hyperdrive::connect」「ConnectError」「KIT_TEST_ADMIN_DATABASE_URL」「kit_rls_probe」「名前付き prepared statement」等。
---

# alc-worker-kit-map — alc-worker-kit 構造ナビゲーション

分割 worker が共通で使う部品の workspace (Refs ippoan/rust-alc-api#723)。直下の `Cargo.toml` が workspace の root
(package なし、`members = ["crates/alc-worker-db"]`、`Cargo.lock` は直下の 1 つ)。toolchain は `rust-toolchain.toml`
(1.92.0 + `wasm32-unknown-unknown`)。deploy・tag は無い (利用側は git 依存の rev 固定)。

| 場所 | 中身 |
|---|---|
| `crates/alc-worker-db/src/lib.rs` | 公開の入口 (`pub use`) と crate の doc。**`compile_fail` の doctest** (`Row` / `Vec<Row>` を `tenant_tx` から返せない・`TenantTx` に `query` / `execute` / `batch_execute` が無い・`PgClient` から生の `Client` を取れない) が、通る版 (`no_run`) と 1 行違いで並ぶ。crate 先頭で `unwrap_used`・`expect_used`・`panic`・`indexing_slicing` を deny (テストは除く) |
| `crates/alc-worker-db/src/tx.rs` | `SET_TENANT` (transaction の頭の文。`set_config('app.current_tenant_id', $1, true)` と search_path `alc_api`) / `PgClient` (`Client` を private に持つ。口は `new`・`current_user` = 固定の文を `simple_query` で 1 回・`tenant_tx` の 3 つ) / `tenant_tx` (`BEGIN` → `SET_TENANT` を `query_typed` で → `f` → `COMMIT`。`f` が `Err` なら drop = ROLLBACK。閉包の Future は `Send`) / `TenantTx<'t>` (`Transaction` を private に持つ。口は `query_typed`・`query_typed_one`・`query_typed_opt`・`execute_typed` の 4 つ) / `TxOutput` (transaction の外へ出してよい owned な値の印。kit の impl は `()`・`bool`・`i16`・`i32`・`i64`・`u64`・`String`・`Uuid`・feature `chrono` の `DateTime<Utc>`・`Option`・`Vec`・2〜4 個のタプル) |
| `crates/alc-worker-db/src/kind.rs` | `kind(&tokio_postgres::Error) -> String`: DB のエラーは SQLSTATE、ほかは表 `PG_KINDS` (`Display` の先頭 12 種 → 固定の label)、当たらなければ `closed` / `other`、原因が `io::Error` なら `:<ErrorKind>`。判定の本体は private の純粋な関数 `label` (unit test 4 本がここ) |
| `crates/alc-worker-db/src/hyperdrive.rs` | **wasm32 専用** (`#[cfg(target_arch = "wasm32")]`。native では compile されない)。`connect(env, binding) -> Result<Option<PgClient>, ConnectError>`: `Reflect::get` が undefined = `Ok(None)` → `env.hyperdrive` → 接続文字列を `Config` に (書き換えない) → StartTls の `Socket` → `connect_raw(socket, PassthroughTls)` → connection を `spawn_local` (Err は `kind` の label だけを `console_error!`)。`ConnectError` は binding 名・段 (`binding`・`config_parse`・`socket`・`handshake`)・`kind` だけ |
| `crates/alc-worker-db/tests/tenant_tx_db.rs` | 実 DB のテスト (native、postgres 16、12 本)。接続先 `KIT_TEST_ADMIN_DATABASE_URL` (準備用の superuser) が**無ければ失敗**。準備 (`PREPARE`。advisory lock で直列) が schema `alc_api`・表 `kit_rls_probe` (RLS は alc-migrations の `vein_templates` と同じ式、FORCE なし)・ロール `kit_test_rt` を作り、本体は `kit_test_rt` で繋ぐ (`rt_connect` が毎回 superuser / BYPASSRLS を落とす)。テナント未設定の transaction の検査だけ、テストが張った素の `Client` を使う |
| `.github/workflows/ci.yml` | job `kit` (postgres 16 の service container): fmt → clippy (native、`--all-features --all-targets`) → clippy (wasm32) → wasm32 のビルド → `cargo llvm-cov --all-features --fail-under-lines 100` (テストの実行を兼ねる) → **走った本数の固定** (unit 4・実 DB 12・0 ignored。テストを足したらここも上げる) → `cargo test --doc` (`compile_fail`)。`auto-merge` は ippoan/ci-workflows の reusable (`checks: read` が要る) |

## 依存

| 依存 | どこで |
|---|---|
| `tokio-postgres` 0.7 (`default-features = false`。wasm32 では feature `js`)・`uuid`・`futures-util` (BoxFuture)・任意 feature `chrono` | 全 target |
| `worker` 0.8 (feature `tokio-postgres`)・`wasm-bindgen`・`wasm-bindgen-futures` | wasm32 だけ (`hyperdrive` module) |
| `tokio`・runtime 付きの `tokio-postgres`・`chrono`・`uuid` の `v4` | native の dev-dependencies (実 DB のテスト) |

`alc-core-wasm` には依存しない。利用側は直下の `[workspace.dependencies]` の 1 か所に git 依存 (rev 固定) を書く (README の「引き方」)。
