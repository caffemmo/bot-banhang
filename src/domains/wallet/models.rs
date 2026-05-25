use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Wallet {
    pub user_id: i64,
    pub balance: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WalletTransaction {
    pub id: i64,
    pub user_id: i64,
    #[sqlx(rename = "type")]
    pub tx_type: String,
    pub amount: i64,
    pub balance_after: i64,
    pub order_id: Option<String>,
    pub topup_id: Option<i64>,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WalletTopupRequest {
    pub id: i64,
    pub user_id: i64,
    pub chat_id: i64,
    pub amount: i64,
    pub memo: String,
    pub status: String,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AdminWalletUser {
    pub user_id: i64,
    pub chat_id: Option<i64>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub full_name: Option<String>,
    pub language_code: Option<String>,
    pub preferred_language: Option<String>,
    pub is_bot: Option<i64>,
    pub user_created_at: Option<String>,
    pub user_updated_at: Option<String>,
    pub balance: i64,
    pub wallet_updated_at: Option<String>,
    pub transaction_count: i64,
    pub last_transaction_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletDetail {
    pub wallet: Wallet,
    pub transactions: Vec<WalletTransaction>,
}

#[derive(Debug, Deserialize)]
pub struct AdjustPayload {
    pub amount: i64,
    pub note: Option<String>,
    pub setup_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ManualTopupPayload {
    pub amount: i64,
    pub note: Option<String>,
    pub setup_code: String,
}
