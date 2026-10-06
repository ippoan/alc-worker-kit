//! `api_error` の各関数が返す status と body の形を固定する。
//! src を写したまま (rust-alc-api d28944e) にしておくため、テストは `tests/` に置く。

use alc_core_wasm::api_error::{
    bad_request, internal_error, internal_error_msg, not_found, unprocessable, upstream_error,
    ApiError,
};
use axum::http::StatusCode;
use serde_json::json;

fn parts(e: ApiError) -> (StatusCode, serde_json::Value) {
    (e.0, e.1 .0)
}

#[test]
fn bad_request_is_400_with_error_and_message() {
    assert_eq!(
        parts(bad_request("invalid_x", "x が不正")),
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": "invalid_x", "message": "x が不正" })
        )
    );
}

#[test]
fn unprocessable_is_422_with_error_and_message() {
    assert_eq!(
        parts(unprocessable("unsupported_format", "未対応の形式")),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": "unsupported_format", "message": "未対応の形式" })
        )
    );
}

#[test]
fn not_found_is_404_with_error_only() {
    assert_eq!(
        parts(not_found("template_not_found")),
        (
            StatusCode::NOT_FOUND,
            json!({ "error": "template_not_found" })
        )
    );
}

#[test]
fn upstream_error_is_502_with_display_as_message() {
    assert_eq!(
        parts(upstream_error("timeout after 5s")),
        (
            StatusCode::BAD_GATEWAY,
            json!({ "error": "upstream_error", "message": "timeout after 5s" })
        )
    );
}

#[test]
fn internal_error_msg_is_500_with_fixed_message() {
    assert_eq!(
        parts(internal_error_msg("設定を読めない")),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "internal_error", "message": "設定を読めない" })
        )
    );
}

/// `STAGING_MODE` を読むのはこの 1 本だけ (環境変数は test binary の中で共有なので、
/// 書き換える検査を 1 つの関数にまとめて並列の取り合いを避ける)。
#[test]
fn internal_error_hides_detail_unless_staging_mode_is_exactly_true() {
    let hidden = (
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": "internal_error" }),
    );

    std::env::remove_var("STAGING_MODE");
    assert_eq!(parts(internal_error("load", "secret detail")), hidden);

    for v in ["1", "TRUE", "yes", ""] {
        std::env::set_var("STAGING_MODE", v);
        assert_eq!(
            parts(internal_error("load", "secret detail")),
            hidden,
            "{v:?}"
        );
    }

    std::env::set_var("STAGING_MODE", "true");
    assert_eq!(
        parts(internal_error("load", "secret detail")),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "internal_error", "context": "load", "detail": "secret detail" })
        )
    );

    std::env::remove_var("STAGING_MODE");
}
