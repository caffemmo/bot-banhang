use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::domains::products::models::Product;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "TEXT")]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    #[sqlx(rename = "pending")]
    Pending,
    #[sqlx(rename = "paid")]
    Paid,
    #[sqlx(rename = "refunded")]
    Refunded,
    #[sqlx(rename = "cancel")]
    Cancel,
    #[sqlx(rename = "expired")]
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "TEXT")]
#[serde(rename_all = "snake_case")]
pub enum OrderReservationMode {
    #[sqlx(rename = "reserved")]
    Reserved,
    #[sqlx(rename = "no_reserve")]
    NoReserve,
}

impl ToString for OrderReservationMode {
    fn to_string(&self) -> String {
        match self {
            OrderReservationMode::Reserved => "reserved",
            OrderReservationMode::NoReserve => "no_reserve",
        }
        .to_string()
    }
}

impl ToString for OrderStatus {
    fn to_string(&self) -> String {
        match self {
            OrderStatus::Pending => "pending",
            OrderStatus::Paid => "paid",
            OrderStatus::Refunded => "refunded",
            OrderStatus::Cancel => "cancel",
            OrderStatus::Expired => "expired",
        }
        .to_string()
    }
}

impl OrderStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "paid" => OrderStatus::Paid,
            "refunded" => OrderStatus::Refunded,
            "cancel" => OrderStatus::Cancel,
            "expired" => OrderStatus::Expired,
            _ => OrderStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub user_id: i64,
    pub chat_id: i64,
    pub product_id: i64,
    pub qty: i64,
    pub amount: i64,
    pub status: OrderStatus,
    pub bank_memo: String,
    pub created_at: String,
    pub paid_at: Option<String>,
    pub payment_tx_id: Option<String>,
    pub delivered_data: Option<String>,
    pub reserved_item_ids: Option<String>,
    pub customer_input: Option<String>,
    pub plan_id: Option<i64>,
    pub plan_label: Option<String>,
    pub plan_months: Option<i64>,
    pub plan_price: Option<i64>,
    pub reservation_mode: OrderReservationMode,
}

impl Order {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        user_id: i64,
        chat_id: i64,
        product_id: i64,
        qty: i64,
        amount: i64,
        bank_memo: String,
        customer_input: Option<String>,
        plan_id: Option<i64>,
        plan_label: Option<String>,
        plan_months: Option<i64>,
        plan_price: Option<i64>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            chat_id,
            product_id,
            qty,
            amount,
            status: OrderStatus::Pending,
            bank_memo,
            created_at: now,
            paid_at: None,
            payment_tx_id: None,
            delivered_data: None,
            reserved_item_ids: None,
            customer_input,
            plan_id,
            plan_label,
            plan_months,
            plan_price,
            reservation_mode: OrderReservationMode::Reserved,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderWithProduct {
    pub order: Order,
    pub product: Product,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct OrderSupportRequest {
    pub id: i64,
    pub order_id: String,
    pub user_id: i64,
    pub chat_id: i64,
    pub username: Option<String>,
    pub product_name: String,
    pub bank_memo: String,
    pub amount: i64,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub id: i64,
    pub received_at: String,
    pub provider: String,
    pub authorized: i64,
    pub source_ip: Option<String>,
    pub memo_extracted: Option<String>,
    pub tx_id: Option<String>,
    pub amount: Option<i64>,
    pub status: Option<String>,
    pub matched_order_id: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub raw_json: Option<String>,
}
