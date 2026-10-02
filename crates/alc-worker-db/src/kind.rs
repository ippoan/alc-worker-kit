//! `tokio_postgres::Error` を、識別子を含まない固定の語に落とす。

use std::error::Error as _;
use std::io;

/// tokio-postgres のエラーの種類。`Display` の先頭 (種類ごとに固定の文) → 固定の label
const PG_KINDS: &[(&str, &str)] = &[
    ("error communicating with the server", "io"),
    ("unexpected message from server", "unexpected_message"),
    ("error performing TLS handshake", "tls"),
    ("error serializing parameter", "to_sql"),
    ("error deserializing column", "from_sql"),
    ("connection closed", "closed"),
    ("error parsing response from server", "parse"),
    ("error encoding message to server", "encode"),
    ("authentication error", "authentication"),
    ("invalid configuration", "config"),
    ("error connecting to server", "connect"),
    ("timeout waiting for server", "timeout"),
];

/// エラーを、識別子を含まない固定の語に落とす: DB のエラーは SQLSTATE、それ以外は tokio-postgres の
/// 種類の label ([`PG_KINDS`]。当たらなければ `closed` / `other`)。原因が `io::Error` なら
/// `:<ErrorKind の名前>` を足す。
/// **`Display` の文は種類の判定に使うだけで、返さない (DB の message も返さない)。**
pub fn kind(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        return db.code().code().to_owned();
    }
    let io = e.source().and_then(|s| s.downcast_ref::<io::Error>());
    label(&e.to_string(), e.is_closed(), io.map(io::Error::kind))
}

/// [`kind`] の判定の本体 (DB のエラー以外)
fn label(display: &str, is_closed: bool, io: Option<io::ErrorKind>) -> String {
    let fallback = if is_closed { "closed" } else { "other" };
    let base = PG_KINDS
        .iter()
        .find(|(prefix, _)| display.starts_with(prefix))
        .map_or(fallback, |(_, label)| label);
    match io {
        Some(io) => format!("{base}:{io:?}"),
        None => base.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表の 12 行の全部。`Display` は「<種類の文>: <原因の文>」の形なので、後ろに文が付いても同じ label
    #[test]
    fn every_table_row_maps_to_its_label() {
        let expected = [
            "io",
            "unexpected_message",
            "tls",
            "to_sql",
            "from_sql",
            "closed",
            "parse",
            "encode",
            "authentication",
            "config",
            "connect",
            "timeout",
        ];
        assert_eq!(PG_KINDS.len(), 12);
        for ((prefix, want), expected) in PG_KINDS.iter().zip(expected) {
            assert_eq!(*want, expected);
            assert_eq!(label(prefix, false, None), expected);
            assert_eq!(label(&format!("{prefix}: 原因の文"), false, None), expected);
        }
    }

    #[test]
    fn unknown_display_falls_back_to_closed_or_other() {
        assert_eq!(label("知らない文", true, None), "closed");
        assert_eq!(label("知らない文", false, None), "other");
        // 表に当たれば is_closed より表が先
        assert_eq!(label("invalid configuration", true, None), "config");
    }

    #[test]
    fn io_source_appends_error_kind_name() {
        let io = Some(io::ErrorKind::ConnectionReset);
        assert_eq!(
            label("error communicating with the server: x", false, io),
            "io:ConnectionReset"
        );
        assert_eq!(label("知らない文", true, io), "closed:ConnectionReset");
        assert_eq!(label("知らない文", false, io), "other:ConnectionReset");
    }

    /// 返すのは固定の語だけで、`Display` の文 (宛先などが入りうる) は含まない
    #[test]
    fn label_never_echoes_the_display_text() {
        let out = label(
            "error connecting to server: db.example.invalid",
            false,
            None,
        );
        assert_eq!(out, "connect");
    }
}
