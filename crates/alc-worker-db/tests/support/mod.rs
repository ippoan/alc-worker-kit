//! 実 DB のテストの準備と接続。`tests/tenant_tx_db.rs` と、crate の中のテスト (`src/tx.rs` の
//! `#[cfg(test)] mod`。private な `Client` を読む検査) の両方が、この 1 つを使う。
//!
//! - **`KIT_TEST_ADMIN_DATABASE_URL` が未設定なら失敗する** (skip して緑にしない)。値は準備用の superuser の
//!   接続文字列で、**使い捨ての DB に向けること** ([`prepare`] が schema `alc_api`・表・ロールを作る)。表示はしない
//! - 準備だけを superuser で流す。**本体のテストは、表の所有者ではない・`NOSUPERUSER`・`NOBYPASSRLS` のロール
//!   [`RT_ROLE`] で繋ぐ** ([`rt_raw_connect`] が毎回 `pg_roles` を読んで確かめる)。
//!   [`RT_PASSWORD`] は使い捨ての DB 専用の固定の文字列で、秘密ではない
//! - 表の RLS は ippoan/alc-migrations の `vein_templates` と同じ式 (`FORCE` なし)
//! - 名前付き prepared statement は使わない (`simple_query`・`batch_execute` だけ)

// 使う側 (2 か所) で、使う項目が違う
#![allow(dead_code)]

use tokio::sync::OnceCell;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls, SimpleQueryMessage};

pub const ADMIN_URL_ENV: &str = "KIT_TEST_ADMIN_DATABASE_URL";
/// 本体のテストが繋ぐロール
pub const RT_ROLE: &str = "kit_test_rt";
/// 使い捨ての DB 専用 (秘密ではない)
pub const RT_PASSWORD: &str = "kit-test-rt-throwaway";

/// 準備。並列に走るテスト (別プロセスを含む) が同時に流しても壊れないよう、advisory lock で直列にする。
/// 何度流しても同じ結果になる書き方 (`IF NOT EXISTS`・在るかを見てから作る)。
pub const PREPARE: &str = r"
BEGIN;
SELECT pg_advisory_xact_lock(723001);
CREATE SCHEMA IF NOT EXISTS alc_api;
CREATE TABLE IF NOT EXISTS alc_api.kit_rls_probe (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL,
    note text NOT NULL,
    at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE alc_api.kit_rls_probe ENABLE ROW LEVEL SECURITY;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT FROM pg_policies
        WHERE schemaname = 'alc_api' AND tablename = 'kit_rls_probe' AND policyname = 'tenant_isolation'
    ) THEN
        CREATE POLICY tenant_isolation ON alc_api.kit_rls_probe
            USING (tenant_id = current_setting('app.current_tenant_id')::uuid);
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'kit_test_rt') THEN
        CREATE ROLE kit_test_rt;
    END IF;
END
$$;
ALTER ROLE kit_test_rt LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'kit-test-rt-throwaway';
GRANT USAGE ON SCHEMA alc_api TO kit_test_rt;
GRANT SELECT, INSERT, UPDATE, DELETE ON alc_api.kit_rls_probe TO kit_test_rt;
COMMIT;
";

/// 準備用の superuser の接続設定。接続文字列は表示しない
pub fn admin_config() -> Config {
    let url = std::env::var(ADMIN_URL_ENV).unwrap_or_else(|_| {
        panic!("{ADMIN_URL_ENV} が未設定。実 DB のテストは接続先が無ければ失敗させる (skip しない)")
    });
    url.parse()
        .unwrap_or_else(|_| panic!("{ADMIN_URL_ENV} が接続文字列として読めない"))
}

pub async fn raw_connect(config: &Config) -> (Client, JoinHandle<()>) {
    let (client, connection) = config
        .connect(NoTls)
        .await
        .unwrap_or_else(|_| panic!("{ADMIN_URL_ENV} の DB に繋げない"));
    let task = tokio::spawn(async move {
        // 接続の終わり方 (テストが切る場合を含む) はここでは見ない
        let _ = connection.await;
    });
    (client, task)
}

/// 準備をプロセスの中で 1 回だけ流す
pub async fn prepare() {
    static PREPARED: OnceCell<()> = OnceCell::const_new();
    PREPARED
        .get_or_init(|| async {
            let (admin, _task) = raw_connect(&admin_config()).await;
            admin.batch_execute(PREPARE).await.unwrap();
        })
        .await;
}

/// [`RT_ROLE`] の素の接続 (admin の接続設定の user と password だけを差し替える)。
/// **ロールが superuser か BYPASSRLS なら panic** (準備のミスで RLS を素通りしたまま緑にしない)。
pub async fn rt_raw_connect() -> (Client, JoinHandle<()>) {
    prepare().await;
    let mut config = admin_config();
    config.user(RT_ROLE).password(RT_PASSWORD);
    let (client, task) = raw_connect(&config).await;
    let flags = first_row(
        &client
            .simple_query(
                "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        flags,
        [Some("f".to_owned()), Some("f".to_owned())],
        "テストの接続ロールが superuser / BYPASSRLS (RLS が効かない)"
    );
    (client, task)
}

/// `simple_query` の最初の行を owned な列にする
pub fn first_row(messages: &[SimpleQueryMessage]) -> Vec<Option<String>> {
    messages
        .iter()
        .find_map(|m| match m {
            SimpleQueryMessage::Row(row) => Some(
                (0..row.len())
                    .map(|i| row.get(i).map(str::to_owned))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap()
}
