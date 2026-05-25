use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Subscriber {
    pub user_id: i64,
    pub chat_id: i64,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub full_name: Option<String>,
    pub language_code: Option<String>,
    pub preferred_language: Option<String>,
    pub stock_notifications_enabled: Option<i64>,
    pub is_bot: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}
