//! `PgClient::tenant_tx`・`TenantTx`・`kind` を、公開の口だけを使って実 DB (postgres 16) で確かめる。
//!
//! ```bash
//! KIT_TEST_ADMIN_DATABASE_URL=postgresql://postgres:<password>@127.0.0.1:<port>/postgres \
//!   cargo test -p alc-worker-db --all-features
//! ```
//!
//! - 接続先・準備・接続ロールは [`support`] (`KIT_TEST_ADMIN_DATABASE_URL` が未設定なら失敗する。
//!   本体のテストは非所有者・`NOBYPASSRLS` のロールで繋ぐ)
//! - 「COMMIT / ROLLBACK の後、**その接続に**設定が残っていない」は、`PgClient` の中の `Client` を読まないと
//!   観測できないので、crate の中のテスト (`src/tx.rs`) に在る
//! - テストごとに乱数の tenant UUID を使い、自分が入れた行は終わりに消す
//! - この crate の規範どおり、テストの中でも名前付き prepared statement は使わない
//!   (`query_typed` 系・`simple_query`・`batch_execute` だけ)

mod support;

use chrono::{DateTime, Utc};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::Type;
use tokio_postgres::{Config, NoTls};
use uuid::Uuid;

use alc_worker_db::{kind, PgClient, SET_TENANT};
use support::{admin_config, first_row, prepare, raw_connect, rt_raw_connect, RT_ROLE};

const INSERT: &str = "INSERT INTO kit_rls_probe (tenant_id, note) VALUES ($1, $2)";
/// WHERE を付けない読み取り (絞るのは RLS だけ)
const SELECT_ALL_TENANTS: &str = "SELECT tenant_id FROM kit_rls_probe";
/// WHERE を付けない削除 (RLS で自テナントの行だけが消える)
const DELETE_ALL: &str = "DELETE FROM kit_rls_probe";
const READ_SETTINGS: &str =
    "SELECT current_setting('app.current_tenant_id', true), current_setting('search_path')";

/// 本体のテストの接続 ([`RT_ROLE`]。superuser / BYPASSRLS なら [`rt_raw_connect`] が panic する)
async fn rt_connect() -> (PgClient, JoinHandle<()>) {
    let (client, task) = rt_raw_connect().await;
    (PgClient::new(client), task)
}

/// この接続のロールの (`rolsuper`, `rolbypassrls`)
async fn role_flags(pg: &mut PgClient) -> (bool, bool) {
    pg.tenant_tx(Uuid::new_v4(), |tx| {
        Box::pin(async move {
            let row = tx
                .query_typed_one(
                    "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
                    &[],
                )
                .await?;
            Ok((row.get(0), row.get(1)))
        })
    })
    .await
    .unwrap()
}

/// tx の中の (`app.current_tenant_id`, `search_path`)
async fn settings(pg: &mut PgClient, tenant: Uuid) -> (Option<String>, String) {
    pg.tenant_tx(tenant, |tx| {
        Box::pin(async move {
            let row = tx.query_typed_one(READ_SETTINGS, &[]).await?;
            Ok((row.get(0), row.get(1)))
        })
    })
    .await
    .unwrap()
}

async fn insert(pg: &mut PgClient, tenant: Uuid, note: &'static str) -> u64 {
    pg.tenant_tx(tenant, move |tx| {
        Box::pin(async move {
            tx.execute_typed(INSERT, &[(&tenant, Type::UUID), (&note, Type::TEXT)])
                .await
        })
    })
    .await
    .unwrap()
}

/// WHERE なしで読めた行の tenant_id
async fn visible_tenants(pg: &mut PgClient, tenant: Uuid) -> Vec<Uuid> {
    pg.tenant_tx(tenant, |tx| {
        Box::pin(async move {
            let rows = tx.query_typed(SELECT_ALL_TENANTS, &[]).await?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
        })
    })
    .await
    .unwrap()
}

/// 自テナントの行を全部消し、消えた行数を返す (WHERE なし。RLS が他テナントの行を守る)
async fn cleanup(pg: &mut PgClient, tenant: Uuid) -> u64 {
    pg.tenant_tx(tenant, |tx| {
        Box::pin(async move { tx.execute_typed(DELETE_ALL, &[]).await })
    })
    .await
    .unwrap()
}

/// `tenant_tx` の Future が `Send` であること (axum の handler が要求する)。compile できれば足りるので呼ばない
#[allow(dead_code)]
fn tenant_tx_future_is_send(pg: &mut PgClient) {
    fn assert_send<T: Send>(_: T) {}
    assert_send(settings(pg, Uuid::nil()));
    assert_send(pg.current_user());
}

/// 1: tx の中で `app.current_tenant_id` が渡した tenant、`search_path` が `alc_api`
#[tokio::test]
async fn tx_sets_tenant_and_search_path() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    assert_eq!(
        settings(&mut pg, tenant).await,
        (Some(tenant.to_string()), "alc_api".to_owned())
    );
}

/// 2a: COMMIT の後、同じ接続の次の tx (別テナント) に前の値が残っていない
#[tokio::test]
async fn next_tx_on_the_same_connection_does_not_see_the_previous_tenant() {
    let (mut pg, _task) = rt_connect().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    assert_eq!(insert(&mut pg, a, "a").await, 1);
    assert_eq!(settings(&mut pg, a).await.0, Some(a.to_string()));

    assert_eq!(settings(&mut pg, b).await.0, Some(b.to_string()));
    assert_eq!(visible_tenants(&mut pg, b).await, Vec::<Uuid>::new());

    assert_eq!(visible_tenants(&mut pg, a).await, vec![a]);
    assert_eq!(cleanup(&mut pg, a).await, 1);
}

/// 2b: `f` が `Err` を返したら COMMIT されず、その tx の INSERT が残らない
#[tokio::test]
async fn err_from_the_closure_rolls_back() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    let result: Result<(), _> = pg
        .tenant_tx(tenant, move |tx| {
            Box::pin(async move {
                let inserted = tx
                    .execute_typed(
                        INSERT,
                        &[(&tenant, Type::UUID), (&"rolled back", Type::TEXT)],
                    )
                    .await?;
                assert_eq!(inserted, 1);
                // 同じ tx の中では見えている
                assert_eq!(tx.query_typed(SELECT_ALL_TENANTS, &[]).await?.len(), 1);
                tx.query_typed("SELECT 1 / 0", &[]).await?;
                Ok(())
            })
        })
        .await;
    assert_eq!(kind(&result.unwrap_err()), "22012");

    // 同じ接続がそのまま使え、行は残っていない
    assert_eq!(visible_tenants(&mut pg, tenant).await, Vec::<Uuid>::new());
    assert_eq!(cleanup(&mut pg, tenant).await, 0);
}

/// 3: RLS だけで他テナントが止まる (WHERE を付けない読み取りが自テナントの行だけ・
/// 他テナントの `tenant_id` での INSERT は 42501)
#[tokio::test]
async fn rls_alone_blocks_another_tenant() {
    let (mut pg, _task) = rt_connect().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    assert_eq!(insert(&mut pg, a, "a1").await, 1);
    assert_eq!(insert(&mut pg, a, "a2").await, 1);
    assert_eq!(insert(&mut pg, b, "b1").await, 1);

    assert_eq!(visible_tenants(&mut pg, a).await, vec![a, a]);
    assert_eq!(visible_tenants(&mut pg, b).await, vec![b]);

    // GUC は b、行の tenant_id は a (policy の USING が INSERT の検査にもなる)
    let result = pg
        .tenant_tx(b, move |tx| {
            Box::pin(async move {
                tx.execute_typed(INSERT, &[(&a, Type::UUID), (&"x", Type::TEXT)])
                    .await
            })
        })
        .await;
    let err = result.unwrap_err();
    assert_eq!(err.code(), Some(&SqlState::INSUFFICIENT_PRIVILEGE));
    assert_eq!(kind(&err), "42501");

    // b からの削除は a の行に届かない
    assert_eq!(cleanup(&mut pg, b).await, 1);
    assert_eq!(visible_tenants(&mut pg, a).await, vec![a, a]);
    assert_eq!(cleanup(&mut pg, a).await, 2);
}

/// 4: テナントを設定していない transaction では表が読めない。kit にその口は無いので、
/// テストが自分で張った素の `tokio_postgres::Client` の `transaction()` で確かめる
#[tokio::test]
async fn transaction_without_tenant_cannot_read() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    // 表が空だと policy の式が評価されないので、行を 1 つ置いておく
    assert_eq!(insert(&mut pg, tenant, "probe").await, 1);

    let (mut raw, _raw_task) = rt_raw_connect().await;
    {
        // 一度も設定していない接続
        let tx = raw.transaction().await.unwrap();
        let err = tx
            .query_typed("SELECT tenant_id FROM alc_api.kit_rls_probe", &[])
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some(&SqlState::UNDEFINED_OBJECT));
    }
    {
        // kit と同じ頭の文で設定して COMMIT
        let tx = raw.transaction().await.unwrap();
        tx.query_typed(SET_TENANT, &[(&tenant.to_string(), Type::TEXT)])
            .await
            .unwrap();
        let rows = tx.query_typed(SELECT_ALL_TENANTS, &[]).await.unwrap();
        assert_eq!(rows.len(), 1);
        tx.commit().await.unwrap();
    }
    {
        // 同じ接続の次の transaction には値も search_path も残っていない
        let tx = raw.transaction().await.unwrap();
        let row = tx.query_typed_one(READ_SETTINGS, &[]).await.unwrap();
        assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some(""));
        assert_ne!(row.get::<_, String>(1), "alc_api");
        let err = tx
            .query_typed("SELECT tenant_id FROM alc_api.kit_rls_probe", &[])
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some(&SqlState::INVALID_TEXT_REPRESENTATION));
    }

    assert_eq!(cleanup(&mut pg, tenant).await, 1);
}

/// 5: `execute_typed` の行数 (0 行・1 行)、`Type::UUID`・`Type::TIMESTAMPTZ`・`Type::TEXT` の引数、
/// `query_typed_one`・`query_typed_opt` (行なし = None)
#[tokio::test]
async fn typed_params_and_row_counts() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    // postgres の timestamptz はマイクロ秒まで
    let at: DateTime<Utc> = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();

    let id: Uuid = pg
        .tenant_tx(tenant, move |tx| {
            Box::pin(async move {
                let row = tx
                    .query_typed_one(
                        "INSERT INTO kit_rls_probe (tenant_id, note, at) VALUES ($1, $2, $3) RETURNING id",
                        &[
                            (&tenant, Type::UUID),
                            (&"before", Type::TEXT),
                            (&at, Type::TIMESTAMPTZ),
                        ],
                    )
                    .await?;
                Ok(row.get(0))
            })
        })
        .await
        .unwrap();

    const UPDATE: &str = "UPDATE kit_rls_probe SET note = $2 WHERE id = $1 AND at = $3";
    let (missed, updated) = pg
        .tenant_tx(tenant, move |tx| {
            Box::pin(async move {
                let other_id = Uuid::new_v4();
                let missed = tx
                    .execute_typed(
                        UPDATE,
                        &[
                            (&other_id, Type::UUID),
                            (&"after", Type::TEXT),
                            (&at, Type::TIMESTAMPTZ),
                        ],
                    )
                    .await?;
                let updated = tx
                    .execute_typed(
                        UPDATE,
                        &[
                            (&id, Type::UUID),
                            (&"after", Type::TEXT),
                            (&at, Type::TIMESTAMPTZ),
                        ],
                    )
                    .await?;
                Ok((missed, updated))
            })
        })
        .await
        .unwrap();
    assert_eq!((missed, updated), (0, 1));

    const READ: &str = "SELECT tenant_id, note, at FROM kit_rls_probe WHERE id = $1";
    type Found = Option<(Uuid, String, i64)>;
    let (found, none): (Found, Option<String>) = pg
        .tenant_tx(tenant, move |tx| {
            Box::pin(async move {
                let found = tx.query_typed_opt(READ, &[(&id, Type::UUID)]).await?;
                let other_id = Uuid::new_v4();
                let none = tx.query_typed_opt(READ, &[(&other_id, Type::UUID)]).await?;
                Ok((
                    found.map(|r| {
                        let read_at: DateTime<Utc> = r.get(2);
                        (r.get(0), r.get(1), read_at.timestamp_micros())
                    }),
                    none.map(|r| r.get(1)),
                ))
            })
        })
        .await
        .unwrap();
    assert_eq!(
        found,
        Some((tenant, "after".to_owned(), at.timestamp_micros()))
    );
    assert_eq!(none, None);

    assert_eq!(cleanup(&mut pg, tenant).await, 1);
}

/// 5 (feature `chrono`): `DateTime<Utc>` をそのまま transaction の外へ返せる
#[cfg(feature = "chrono")]
#[tokio::test]
async fn timestamptz_round_trips_as_tx_output() {
    let (mut pg, _task) = rt_connect().await;
    let at: DateTime<Utc> = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();
    let echoed: Option<DateTime<Utc>> = pg
        .tenant_tx(Uuid::new_v4(), move |tx| {
            Box::pin(async move {
                let row = tx
                    .query_typed_one("SELECT $1::timestamptz", &[(&at, Type::TIMESTAMPTZ)])
                    .await?;
                Ok(row.get(0))
            })
        })
        .await
        .unwrap();
    assert_eq!(echoed, Some(at));
}

/// 5 (追加): `Type::TEXT_ARRAY` (`= ANY($1)` に `Vec<String>`) と `Type::INT4` (`i32`) の引数が、
/// `query_typed` と `execute_typed` の両方で通る
#[tokio::test]
async fn text_array_and_int4_params() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    for note in ["a", "bb", "ccc"] {
        assert_eq!(insert(&mut pg, tenant, note).await, 1);
    }

    let wanted = vec!["a".to_owned(), "ccc".to_owned(), "none".to_owned()];
    let (read, deleted): (Vec<(String, i32)>, u64) = pg
        .tenant_tx(tenant, move |tx| {
            Box::pin(async move {
                let rows = tx
                    .query_typed(
                        "SELECT note, length(note) + $2 FROM kit_rls_probe WHERE note = ANY($1) ORDER BY note",
                        &[(&wanted, Type::TEXT_ARRAY), (&10_i32, Type::INT4)],
                    )
                    .await?;
                let deleted = tx
                    .execute_typed(
                        "DELETE FROM kit_rls_probe WHERE note = ANY($1) AND length(note) = $2",
                        &[(&wanted, Type::TEXT_ARRAY), (&3_i32, Type::INT4)],
                    )
                    .await?;
                Ok((rows.iter().map(|r| (r.get(0), r.get(1))).collect(), deleted))
            })
        })
        .await
        .unwrap();
    assert_eq!(read, [("a".to_owned(), 11), ("ccc".to_owned(), 13)]);
    assert_eq!(deleted, 1);

    // 残りは "a" と "bb"
    assert_eq!(cleanup(&mut pg, tenant).await, 2);
}

/// 2 (追加): 1 つの `PgClient` で `tenant_tx` を続けて 200 回開ける (接続を張り直さない)。
/// テナントを交互に替え、毎回 tx の中の値が渡した tenant
#[tokio::test]
async fn one_client_opens_200_transactions_in_a_row() {
    let (mut pg, _task) = rt_connect().await;
    let tenants = [Uuid::new_v4(), Uuid::new_v4()];
    for n in 0..200 {
        let tenant = tenants[n % 2];
        assert_eq!(
            settings(&mut pg, tenant).await,
            (Some(tenant.to_string()), "alc_api".to_owned()),
            "{n} 回目"
        );
    }
}

/// 無ければ入れ、在れば書き換える。その後、同じ tx の中でもう 1 文流す。
/// 戻り値は (入れたか, 1 つ目の書き込みの行数, 2 つ目の書き込みの行数)
async fn put_note(pg: &mut PgClient, tenant: Uuid) -> (bool, u64, u64) {
    pg.tenant_tx(tenant, move |tx| {
        Box::pin(async move {
            let found = tx
                .query_typed(
                    "SELECT id FROM kit_rls_probe WHERE note = $1",
                    &[(&"branch", Type::TEXT)],
                )
                .await?;
            // 途中の結果で分岐する
            let (inserted, first) = match found.first() {
                None => {
                    let n = tx
                        .execute_typed(INSERT, &[(&tenant, Type::UUID), (&"branch", Type::TEXT)])
                        .await?;
                    (true, n)
                }
                Some(row) => {
                    let id: Uuid = row.get(0);
                    let n = tx
                        .execute_typed(
                            "UPDATE kit_rls_probe SET at = now() WHERE id = $1",
                            &[(&id, Type::UUID)],
                        )
                        .await?;
                    (false, n)
                }
            };
            let second = tx
                .execute_typed(
                    "UPDATE kit_rls_probe SET note = note WHERE note = $1",
                    &[(&"branch", Type::TEXT)],
                )
                .await?;
            Ok((inserted, first, second))
        })
    })
    .await
    .unwrap()
}

/// 2 (追加): 1 transaction の中で `query_typed` → `execute_typed` → `execute_typed` を順に await し、
/// 途中の結果で分岐する形
#[tokio::test]
async fn several_statements_in_one_transaction_with_a_branch() {
    let (mut pg, _task) = rt_connect().await;
    let tenant = Uuid::new_v4();
    // 1 回目は行が無いので入れる側、2 回目は在るので書き換える側
    assert_eq!(put_note(&mut pg, tenant).await, (true, 1, 1));
    assert_eq!(put_note(&mut pg, tenant).await, (false, 1, 1));
    assert_eq!(cleanup(&mut pg, tenant).await, 1);
}

/// 6: 並列。接続 8 本を同時に、テナント 2 つを交互に各 50 tx。各 tx で自テナントの行を 1 つ入れ、
/// WHERE なしで読んだ行が全部自テナント
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_connections_never_see_another_tenant() {
    const CONNECTIONS: usize = 8;
    const TXS: usize = 50;
    let tenants = [Uuid::new_v4(), Uuid::new_v4()];

    let mut workers = Vec::new();
    for i in 0..CONNECTIONS {
        let tenant = tenants[i % 2];
        workers.push(tokio::spawn(async move {
            let (mut pg, _task) = rt_connect().await;
            for n in 0..TXS {
                let seen: Vec<Uuid> = pg
                    .tenant_tx(tenant, move |tx| {
                        Box::pin(async move {
                            tx.execute_typed(
                                INSERT,
                                &[(&tenant, Type::UUID), (&"parallel", Type::TEXT)],
                            )
                            .await?;
                            let rows = tx.query_typed(SELECT_ALL_TENANTS, &[]).await?;
                            Ok(rows.iter().map(|r| r.get(0)).collect())
                        })
                    })
                    .await
                    .unwrap();
                // 少なくとも、この接続がここまでに入れた行は見えている
                assert!(seen.len() > n);
                assert!(seen.iter().all(|t| *t == tenant), "他テナントの行が見えた");
            }
        }));
    }
    for worker in workers {
        worker.await.unwrap();
    }

    let (mut pg, _task) = rt_connect().await;
    let per_tenant = (CONNECTIONS / 2 * TXS) as u64;
    for tenant in tenants {
        assert_eq!(cleanup(&mut pg, tenant).await, per_tenant);
    }
}

/// 7: 本体のテストの接続ロールは superuser でも BYPASSRLS でもなく、表の所有者でもない。
/// 準備用の接続 (superuser) を同じ検査に掛けると検出できる (検査が効いていることの対照)
#[tokio::test]
async fn runtime_role_is_not_privileged() {
    let (mut pg, _task) = rt_connect().await;
    assert_eq!(role_flags(&mut pg).await, (false, false));
    let owned_by_me: bool = pg
        .tenant_tx(Uuid::new_v4(), |tx| {
            Box::pin(async move {
                let row = tx
                    .query_typed_one(
                        "SELECT tableowner = current_user FROM pg_tables WHERE schemaname = 'alc_api' AND tablename = 'kit_rls_probe'",
                        &[],
                    )
                    .await?;
                Ok(row.get(0))
            })
        })
        .await
        .unwrap();
    assert!(!owned_by_me);

    let (admin, _admin_task) = raw_connect(&admin_config()).await;
    let (rolsuper, _) = role_flags(&mut PgClient::new(admin)).await;
    assert!(rolsuper, "準備用の接続は superuser のはず");
}

/// 8: `current_user()` が接続のロール名
#[tokio::test]
async fn current_user_is_the_runtime_role() {
    let (pg, _task) = rt_connect().await;
    assert_eq!(pg.current_user().await.unwrap().as_deref(), Some(RT_ROLE));
}

/// 0 除算 (22012) を起こし、失敗を [`kind`] の語で返す
async fn divide_by_zero(pg: &mut PgClient) -> Result<(), String> {
    pg.tenant_tx(Uuid::new_v4(), |tx| {
        Box::pin(async move {
            tx.query_typed("SELECT 1 / 0", &[]).await?;
            Ok(())
        })
    })
    .await
    .map_err(|e| kind(&e))
}

/// 9: `kind` — DB のエラーは SQLSTATE、切れた接続は固定の label、原因が io なら `ErrorKind` の名前が付く
#[tokio::test]
async fn kind_maps_db_errors_and_closed_connections() {
    let (mut pg, task) = rt_connect().await;
    assert_eq!(divide_by_zero(&mut pg).await, Err("22012".to_owned()));

    // 接続の task を止める = 接続が切れる
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(divide_by_zero(&mut pg).await, Err("closed".to_owned()));
    let err = pg.current_user().await.unwrap_err();
    assert_eq!(kind(&err), "closed");

    // 誰も聞いていないポートへの接続 (原因が io::Error)
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = Config::new();
    config.host("127.0.0.1").port(port).user(RT_ROLE);
    let err = config.connect(NoTls).await.err().unwrap();
    assert_eq!(kind(&err), "connect:ConnectionRefused");
}

/// 準備が作ったロールの属性 (`simple_query` で読む。superuser の接続)
#[tokio::test]
async fn prepared_role_has_login_without_bypass() {
    prepare().await;
    let (admin, _task) = raw_connect(&admin_config()).await;
    let messages = admin
        .simple_query(
            "SELECT rolcanlogin, rolsuper, rolbypassrls FROM pg_roles WHERE rolname = 'kit_test_rt'",
        )
        .await
        .unwrap();
    let row = first_row(&messages);
    let flags: Vec<&str> = row.iter().map(|c| c.as_deref().unwrap()).collect();
    assert_eq!(flags, ["t", "f", "f"]);
}
