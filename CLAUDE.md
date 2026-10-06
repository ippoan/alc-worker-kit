# alc-worker-kit

分割 worker (Cloudflare Workers) が共通で使う部品の workspace。在るのは DB の部品 `crates/alc-worker-db`
(Refs ippoan/rust-alc-api#723) と、テナントのヘッダの layer・型・エラーの `crates/alc-core-wasm`
(rust-alc-api から移した。Refs ippoan/rust-alc-api#736) の 2 つ。構造は `.claude/skills/alc-worker-kit-map`、詳細は `README.md`。

- 直下 = workspace の root (package なし。`Cargo.lock` は直下の 1 つ)
- `crates/alc-worker-db/` = `PgClient`・`TenantTx`・`TxOutput`・`SET_TENANT`・`kind`・`hyperdrive::connect` (wasm32 専用)
- `crates/alc-core-wasm/` = `require_tenant_header`・`TenantId`・`AuthUser`・`DbError`・`api_error`・`device_dev` (native と wasm32 の両方。feature `sqlx` は任意)

## コマンド

```bash
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo clippy --locked --target wasm32-unknown-unknown --all-features -- -D warnings
cargo build --locked --target wasm32-unknown-unknown --all-features
# 実 DB のテスト + 行カバレッジ 100% (使い捨ての postgres 17 を空きポートで立ててから。README の「検査の回し方」)
KIT_TEST_ADMIN_DATABASE_URL=... cargo llvm-cov --locked --all-features --fail-under-lines 100
cargo test --locked --doc --all-features   # compile_fail の doctest
```

## 規範

- **口を足さない。** 名前付きの文を呼ぶ口 (`query`・`execute`・`prepare`)・任意の SQL を流す口 (`simple_query`・`batch_execute`)・
  テナントを設定しない transaction を作る口・生の `Client` / `Transaction` へ降りる口 (`Deref`・`AsRef`・`into_inner`・`client()`) を、
  `PgClient` にも `TenantTx` にも足さない。公開する型・メソッドは README の表が全部。
- **kit の中でも名前付き prepared statement を呼ばない** (テストを含む)。使うのは `query_typed` 系・`execute_typed`・固定の文の `simple_query`
  (テストの準備の superuser の接続だけ `batch_execute`)。
  `git grep -nE '^[^/]*\.(execute|query|query_one|query_opt|prepare|batch_execute)\(' -- crates/alc-worker-db/src` が 0 行。
- **テナント分離を弱めない。** `tenant_tx` の順 (`BEGIN` → `SET_TENANT` → `f` → `COMMIT`) と `SET_TENANT` の文 (第 3 引数 `true`) を変えない。
  `TxOutput` を `Row`・`Statement`・`RowStream` に付けない。`compile_fail` の doctest を外さない。
- **実 DB のテストを skip にしない。** 接続先 (`KIT_TEST_ADMIN_DATABASE_URL`) が無ければ失敗する作りのまま。`#[ignore]` を付けない。
  本体のテストは非所有者・`NOBYPASSRLS` のロールで繋ぐ (準備と接続は `tests/support/mod.rs` の 1 つ)。
  `PgClient` の中の `Client` を読む検査は `src/tx.rs` の `#[cfg(test)] mod tests` に置き、そのために口 (`into_inner`・テスト用の feature) を足さない。行カバレッジ 100% の gate と、CI の本数の固定を緩めない
  (テストを足したら `ci.yml` の本数を上げる)。
- **エラーの文を出さない。** `kind` と `ConnectError` が出すのは固定の label・SQLSTATE・`io::ErrorKind` の名前だけ。
  `Display` の文・DB の message・JS のエラー文・接続文字列・ホスト名を、戻り値・`Debug`・ログに出さない。
- **`alc-core-wasm` は kit 内の別 crate として同居し、`alc-worker-db` は `alc-core-wasm` に依存しない。**
  利用側で出どころが 2 つになると (kit と rust-alc-api の両方から引く等) 型が分かれ、コンパイルは通るのに全リクエストが 500 になる。
  `alc-core-wasm` の src は rust-alc-api d28944e から写したもの (差は tests module の 2 か所 (`device_dev.rs` の、失敗時メッセージ引数が別の行に在った `assert!`) だけを 1 行の `assert!` に書き換えた — 行カバレッジ 100% の gate のため。本体のコードは写したまま)。テストは `crates/alc-core-wasm/tests/` に足す。
- **public repo。** ホスト名・IP・account ID・テナント ID・接続文字列・Hyperdrive の設定の ID の実物を、コード・コメント・commit・PR に書かない。
- **tag を打たない。** 利用側は git 依存の rev 固定で引く。deploy の workflow・repo 単位の secret を足さない。
- 手元の検査の DB は別名・空きポート (`-p 127.0.0.1::5432`) の使い捨てコンテナ。`docker ps` に出ているほかのコンテナには繋がない・止めない。
