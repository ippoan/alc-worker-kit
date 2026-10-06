//! wasm32 (Cloudflare Workers) でもビルドできる `alc-core` の共有部分。
//!
//! sqlx / reqwest / ring / tokio に依存する部分は `alc-core` に残す。
//! `alc-core` は本 crate の項目を元のパスのまま re-export する。

pub mod api_error;
pub mod db_error;
pub mod device_dev;
pub mod tenant_header;
pub mod types;

pub use db_error::DbError;
pub use device_dev::{device_dev_from_headers, device_tenko_manager_from_headers, DeviceDevSlot};
pub use tenant_header::require_tenant_header;
pub use types::{AuthUser, TenantId};
