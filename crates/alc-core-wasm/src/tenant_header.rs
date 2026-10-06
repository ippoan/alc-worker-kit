use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use uuid::Uuid;

use crate::device_dev::{
    device_dev_from_headers, device_tenko_manager_from_headers, DeviceDevSlot,
};
use crate::types::{AuthUser, TenantId};

/// 注入された identity ヘッダーを信頼するミドルウェア (Refs #434)
///
/// **前段の trusted proxy (CF Worker = alc-app / carins / nuxt-items、または
/// per-domain API gateway) が auth-worker `/auth/introspect` で user/device JWT を
/// 検証し、検証済み identity を `X-Tenant-ID` / `X-User-ID` / `X-User-Email` /
/// `X-User-Role` ヘッダーとして注入している前提**。rust-alc-api 自身は JWT 検証を
/// 行わず、注入された identity を信頼するだけの dumb backend に徹する。
///
/// #434 で monolith の `require_jwt` / `require_tenant` (= ローカル JWT 検証 +
/// bare X-Tenant-ID フォールバック) を撤去し、tenant/admin 経路をこのミドルウェアに
/// 一本化した。外部からの直叩き防止は **Cloud Run IAM による網層ロックダウン**
/// (proxy の OIDC ID token のみ到達可) が担う (= 確定アーキ #4807535677、step 3)。
///
/// - `X-Tenant-ID` 欠落 → 401
/// - `X-User-ID` / `X-User-Email` / `X-User-Role` が揃えば AuthUser も復元する
///   (admin 経路の role 判定はハンドラ側が AuthUser から行う)
/// - 呼び元が request extensions に `DeviceDevSlot` を入れていれば、認証を通した後に
///   `X-Device-Dev` の印と、`X-Device-Role` が運行管理者用の鍵かを書く
///   (Refs ippoan/alc-app#387)。入れ物が無ければ何もしない
pub async fn require_tenant_header(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let tenant_id = req
        .headers()
        .get("X-Tenant-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    req.extensions_mut().insert(TenantId(tenant_id));

    if let Some(slot) = req.extensions().get::<DeviceDevSlot>() {
        slot.set(device_dev_from_headers(req.headers()));
        slot.set_tenko_manager(device_tenko_manager_from_headers(req.headers()));
    }

    // Gateway が注入した認証ヘッダーから AuthUser を復元
    let user_id = req
        .headers()
        .get("X-User-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok());
    let email = req
        .headers()
        .get("X-User-Email")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let role = req
        .headers()
        .get("X-User-Role")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let tenant_slug = req
        .headers()
        .get("X-Tenant-Slug")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    if let (Some(user_id), Some(email), Some(role)) = (user_id, email, role) {
        let auth_user = AuthUser {
            user_id,
            email,
            name: String::new(),
            tenant_id,
            tenant_slug,
            role,
        };
        req.extensions_mut().insert(auth_user);
    }

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware as axum_middleware, routing::get, Extension, Router};
    use tower::ServiceExt;

    async fn echo_tenant(Extension(tid): Extension<TenantId>) -> String {
        tid.0.to_string()
    }

    async fn echo_auth_user(Extension(user): Extension<AuthUser>) -> String {
        format!("{}:{}:{:?}", user.email, user.role, user.tenant_slug)
    }

    fn app() -> Router {
        Router::new()
            .route("/t", get(echo_tenant))
            .route("/u", get(echo_auth_user))
            .layer(axum_middleware::from_fn(require_tenant_header))
    }

    async fn send(headers: &[(&str, &str)], uri: &str) -> Response {
        let mut b = Request::builder().uri(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        app()
            .into_service()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_string(resp: Response) -> String {
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        String::from_utf8_lossy(&body).into_owned()
    }

    /// 入れ物を request extensions に入れてから middleware を通し、書かれた値を返す。
    async fn slot_after(headers: &[(&str, &str)]) -> (StatusCode, bool) {
        let slot = DeviceDevSlot::default();
        let mut b = Request::builder().uri("/t");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut req = b.body(Body::empty()).unwrap();
        req.extensions_mut().insert(slot.clone());
        let resp = app().into_service().oneshot(req).await.unwrap();
        (resp.status(), slot.get())
    }

    #[tokio::test]
    async fn device_dev_written_to_slot_after_auth() {
        let tid = Uuid::new_v4().to_string();
        let got = slot_after(&[("X-Tenant-ID", &tid), ("X-Device-Dev", "1")]).await;
        assert_eq!(got, (StatusCode::OK, true));
        for v in ["true", "0", ""] {
            let got = slot_after(&[("X-Tenant-ID", &tid), ("X-Device-Dev", v)]).await;
            assert_eq!(got, (StatusCode::OK, false), "{v:?}");
        }
        let got = slot_after(&[("X-Tenant-ID", &tid)]).await;
        assert_eq!(got, (StatusCode::OK, false));
    }

    #[tokio::test]
    async fn device_dev_not_written_when_auth_fails() {
        let got = slot_after(&[("X-Device-Dev", "1")]).await;
        assert_eq!(got, (StatusCode::UNAUTHORIZED, false));
    }

    /// `slot_after` と同じ形で、運行管理者の鍵かの印まで返す。
    async fn slot_marks_after(headers: &[(&str, &str)]) -> (StatusCode, bool, bool) {
        let slot = DeviceDevSlot::default();
        let mut b = Request::builder().uri("/t");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut req = b.body(Body::empty()).unwrap();
        req.extensions_mut().insert(slot.clone());
        let resp = app().into_service().oneshot(req).await.unwrap();
        (resp.status(), slot.get(), slot.is_tenko_manager())
    }

    #[tokio::test]
    async fn device_role_written_to_slot_after_auth() {
        let tid = Uuid::new_v4().to_string();
        let role = ("X-Device-Role", "device-tenko-manager");
        let got = slot_marks_after(&[("X-Tenant-ID", &tid), role]).await;
        assert_eq!(got, (StatusCode::OK, false, true));
        let got = slot_marks_after(&[("X-Tenant-ID", &tid), ("X-Device-Dev", "1"), role]).await;
        assert_eq!(got, (StatusCode::OK, true, true));
        for v in ["device-kiosk", "Device-Tenko-Manager", ""] {
            let got = slot_marks_after(&[("X-Tenant-ID", &tid), ("X-Device-Role", v)]).await;
            assert_eq!(got, (StatusCode::OK, false, false), "{v:?}");
        }
        let got = slot_marks_after(&[("X-Tenant-ID", &tid), ("X-Device-Dev", "1")]).await;
        assert_eq!(got, (StatusCode::OK, true, false));
    }

    #[tokio::test]
    async fn device_role_not_written_when_auth_fails() {
        let got = slot_marks_after(&[
            ("X-Device-Dev", "1"),
            ("X-Device-Role", "device-tenko-manager"),
        ])
        .await;
        assert_eq!(got, (StatusCode::UNAUTHORIZED, false, false));
    }

    #[tokio::test]
    async fn device_dev_header_without_slot_is_ignored() {
        let tid = Uuid::new_v4();
        let resp = send(
            &[("X-Tenant-ID", &tid.to_string()), ("X-Device-Dev", "1")],
            "/t",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tenant_header_ok() {
        let tid = Uuid::new_v4();
        let resp = send(&[("X-Tenant-ID", &tid.to_string())], "/t").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_string(resp).await, tid.to_string());
    }

    #[tokio::test]
    async fn tenant_header_missing() {
        let resp = send(&[], "/t").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tenant_header_invalid_uuid() {
        let resp = send(&[("X-Tenant-ID", "not-a-uuid")], "/t").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tenant_header_with_auth_user() {
        let tid = Uuid::new_v4();
        let uid = Uuid::new_v4();
        let resp = send(
            &[
                ("X-Tenant-ID", &tid.to_string()),
                ("X-User-ID", &uid.to_string()),
                ("X-User-Email", "test@example.com"),
                ("X-User-Role", "admin"),
                ("X-Tenant-Slug", "acme"),
            ],
            "/u",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            body_string(resp).await,
            "test@example.com:admin:Some(\"acme\")"
        );
    }

    #[tokio::test]
    async fn partial_user_headers_do_not_restore_auth_user() {
        let tid = Uuid::new_v4();
        let resp = send(
            &[
                ("X-Tenant-ID", &tid.to_string()),
                ("X-User-ID", &Uuid::new_v4().to_string()),
            ],
            "/u",
        )
        .await;
        // AuthUser が復元されないので Extension 抽出が失敗する (500)
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
