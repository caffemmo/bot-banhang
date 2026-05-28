use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub price: i64,
    pub is_active: Option<i64>,
    pub requires_input: Option<i64>,
    pub input_prompt: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub delivery_type: Option<String>,
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub file_mime: Option<String>,
    pub category_id: Option<i64>,
    pub category: Option<String>,
    pub category_emoji: Option<String>,
    pub category_custom_emoji_id: Option<String>,
    pub button_emoji: Option<String>,
    pub button_custom_emoji_id: Option<String>,
    pub created_at: Option<String>,
    pub sort_order: Option<i64>,
    pub show_sold_count: Option<i64>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProductCategory {
    pub id: i64,
    pub name: String,
    pub emoji: Option<String>,
    pub custom_emoji_id: Option<String>,
    pub sort_order: Option<i64>,
    pub is_active: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProductPlan {
    pub id: i64,
    pub product_id: i64,
    pub label: String,
    pub months: i64,
    pub price: i64,
    pub sort_order: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProductItem {
    pub id: i64,
    pub product_id: i64,
    pub content: String,
    pub created_at: Option<String>,
    pub is_buy: Option<i64>,
}
