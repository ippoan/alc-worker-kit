//! ハンドラ共通の「理由付きエラーレスポンス」ヘルパー。
//!
//! 同型の定義が `alc-auth::internal` (`ErrorResponse`, `internal_error` /
//! `not_found`) / `alc-notify::lineworks_channels` (`ApiError`,
//! `bad_request` 等) / `alc-notify::lineworks_config` (`internal_error`) の
//! 3 箇所に散っていたのをここへ統合した (Refs ippoan/rust-alc-api#668)。
//! 呼び出し側は本モジュールの関数を呼ぶだけで、独自定義を増やさないこと。

use axum::{http::StatusCode, Json};

/// ハンドラの共通エラー型: `(StatusCode, Json<{"error": ...}>)`。
pub type ApiError = (StatusCode, Json<serde_json::Value>);
pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// 理由付き 400。`error` は機械可読な短いコード、`message` は人間向けの説明。
pub fn bad_request(error: &str, message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": error, "message": message })),
    )
}

/// 理由付き 422。形は正しい JSON だが中身を処理できない (例: 指静脈の特徴量が
/// 未対応の形式) ときに使う。`error` / `message` は `bad_request` と同じ。
pub fn unprocessable(error: &str, message: &str) -> ApiError {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({ "error": error, "message": message })),
    )
}

/// 理由付き 404 (`error` のみ)。
pub fn not_found(error: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": error })),
    )
}

/// 上流 (外部 API) エラー。502。
pub fn upstream_error(e: impl std::fmt::Display) -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({ "error": "upstream_error", "message": e.to_string() })),
    )
}

/// 内部エラー (詳細付き)。staging (揮発 DB) では `detail` を response に載せて
/// 診断を高速化し、本番では文言を隠す (情報漏洩防止。alc-auth::internal の元実装を
/// 踏襲)。
pub fn internal_error(context: &str, err: impl std::fmt::Display) -> ApiError {
    let detail = err.to_string();
    tracing::error!("internal error ({context}): {detail}");
    let body = if std::env::var("STAGING_MODE").as_deref() == Ok("true") {
        serde_json::json!({ "error": "internal_error", "context": context, "detail": detail })
    } else {
        serde_json::json!({ "error": "internal_error" })
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body))
}

/// 内部エラー (固定メッセージのみ)。呼び出し側で既に `tracing::error!` 済みで、
/// response には固定メッセージだけを載せたい場合向け
/// (alc-notify::lineworks_channels / lineworks_config の元実装を踏襲)。
pub fn internal_error_msg(msg: &str) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal_error", "message": msg })),
    )
}
