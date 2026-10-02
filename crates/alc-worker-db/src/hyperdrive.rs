//! Hyperdrive の binding から DB へ繋ぐ (wasm32 専用。native では compile されない)。
//!
//! 繋ぎ方は workers-rs の公式例 (examples/tokio-postgres) の形: binding の接続文字列を `Config` にし
//! (**書き換えない**)、binding の host:port へ StartTls の `Socket` を開いて `connect_raw` に渡す。
//!
//! **エラーの文・宛先・接続文字列は、`Display`・`Debug`・ログのどこにも出さない。** 出すのは binding 名と、
//! 段の固定の label と、[`crate::kind`] の語だけ。

use std::fmt;

use tokio_postgres::Config;
use wasm_bindgen::JsValue;
use worker::js_sys::Reflect;
use worker::postgres_tls::PassthroughTls;
use worker::{console_error, Env, SecureTransport, Socket};

use crate::{kind, PgClient};

/// [`connect`] の失敗。持つのは binding 名・段の label・`kind` だけ (文を持たない)。
///
/// 段は `binding` (読めない・Hyperdrive の binding ではない)・`config_parse`・`socket`・`handshake`。
/// `Display` は `hyperdrive <binding>: <段>` か `hyperdrive <binding>: handshake: <kind>`。
#[derive(Debug)]
pub struct ConnectError {
    binding: String,
    stage: &'static str,
    kind: Option<String>,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hyperdrive {}: {}", self.binding, self.stage)?;
        match &self.kind {
            Some(kind) => write!(f, ": {kind}"),
            None => Ok(()),
        }
    }
}

/// Hyperdrive の binding `binding` から繋ぐ。
/// - binding が無い (**undefined のときだけ**。null は「在る」) → `Ok(None)` (呼び手が次の段へ進める)
/// - 在るのに使えない (読めない・型違い・接続文字列が parse できない・socket・handshake) → `Err`
/// - 繋がった → `Ok(Some)`
///
/// `env.hyperdrive()` の Err では「無い」を判定しない: Err は binding が在って型の照合に落ちたときにも
/// 返るので、「無い」とみなすと、binding が在るのに別の段へ黙って落ちる。
pub async fn connect(env: &Env, binding: &str) -> Result<Option<PgClient>, ConnectError> {
    let fail = |stage: &'static str, kind: Option<String>| ConnectError {
        binding: binding.to_owned(),
        stage,
        kind,
    };
    let value = Reflect::get(env, &JsValue::from(binding)).map_err(|_| fail("binding", None))?;
    if value.is_undefined() {
        return Ok(None);
    }
    let hd = env.hyperdrive(binding).map_err(|_| fail("binding", None))?;
    let config = hd
        .connection_string()
        .parse::<Config>()
        .map_err(|_| fail("config_parse", None))?;
    let socket = Socket::builder()
        .secure_transport(SecureTransport::StartTls)
        .connect(hd.host(), hd.port())
        .map_err(|_| fail("socket", None))?;
    let (client, connection) = config
        .connect_raw(socket, PassthroughTls)
        .await
        .map_err(|e| fail("handshake", Some(kind(&e))))?;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = connection.await {
            console_error!("alc-worker-db: connection task: {}", kind(&e));
        }
    });
    Ok(Some(PgClient::new(client)))
}
