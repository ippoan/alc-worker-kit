use uuid::Uuid;

/// 認証済みユーザー情報 (JWT から抽出)
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub tenant_id: Uuid,
    pub tenant_slug: Option<String>,
    pub role: String,
}

/// テナント ID (後方互換)
#[derive(Debug, Clone, Copy)]
pub struct TenantId(pub Uuid);
