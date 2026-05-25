use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Conversation {
    pub user_id: i64,
    pub chat_id: i64,
    pub username: Option<String>,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub updated_at: Option<String>,
    pub last_activity_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ChatMessage {
    pub id: i64,
    pub chat_id: i64,
    pub user_id: Option<i64>,
    pub direction: String,
    pub text: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub telegram_date: Option<String>,
    pub raw_json: Option<String>,
    pub created_at: Option<String>,
}
