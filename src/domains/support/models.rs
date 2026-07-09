use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SupportTicket {
    pub id: i64,
    pub public_key: String,
    pub kind: String,
    pub status: String,
    pub customer_name: Option<String>,
    pub contact_method: String,
    pub contact_value: Option<String>,
    pub order_ref: Option<String>,
    pub facebook_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SupportMessage {
    pub id: i64,
    pub ticket_id: i64,
    pub sender: String,
    pub message: String,
    pub created_at: String,
}

pub fn support_kind_label(kind: &str) -> &'static str {
    match kind {
        "order" => "Hỗ trợ đơn hàng",
        "meta_verified" => "Hỗ trợ lên tích xanh",
        "facebook_unlock" => "Hỗ trợ mở khóa Facebook",
        _ => "Hỗ trợ",
    }
}
