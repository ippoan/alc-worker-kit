//! 分割 worker (Cloudflare Workers) が共通で使う暗号の部品 (Refs ippoan/rust-alc-api#747)。
//!
//! - [`secret`] — DB に置く秘密値 (LINE の channel secret / private key、LINE WORKS の client secret など) の
//!   AES-256-GCM の暗号化と復号。鍵は SHA-256(`SSO_ENCRYPTION_KEY`)。rust-alc-api の
//!   `alc_core::auth_lineworks` (ring) と同じ形式で、相互に読める
//! - [`jwt`] — RS256 の JWT の署名 (header に `kid` を任意で足す)。claim は呼び手の型
//!
//! pure Rust (RustCrypto) で、native でも wasm32-unknown-unknown でもビルドできる。
//! エラーは段の名前だけを持つ (暗号文・鍵・平文を `Display` / `Debug` に入れない)。
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod jwt;
pub mod secret;
