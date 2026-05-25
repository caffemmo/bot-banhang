use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AdminUser {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserPublic {
    pub id: i64,
    pub username: String,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

impl From<AdminUser> for AdminUserPublic {
    fn from(value: AdminUser) -> Self {
        Self {
            id: value.id,
            username: value.username,
            is_active: value.is_active,
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_login_at: value.last_login_at,
        }
    }
}
