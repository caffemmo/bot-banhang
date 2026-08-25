use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use teloxide::prelude::*;
use teloxide::types::ChatId;
use tracing::warn;
use uuid::Uuid;

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};

const MAX_ITEM_QUANTITY: u16 = 20;
const MAX_TOTAL_QUANTITY: u16 = 30;

#[derive(Debug, Deserialize)]
pub struct CreateOrderPayload {
    pub customer_name: String,
    pub phone: String,
    pub province: String,
    pub ward: String,
    pub address: String,
    pub delivery_note: Option<String>,
    pub items: Vec<OrderItemPayload>,
    #[serde(default)]
    pub website: String,
}

#[derive(Debug, Deserialize)]
pub struct OrderItemPayload {
    pub sku: String,
    pub quantity: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct StoredOrderItem {
    sku: String,
    name: &'static str,
    quantity: u16,
}

#[derive(Debug, Serialize)]
pub struct CreateOrderResponse {
    pub order_id: String,
    pub received_at: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PhysicalOrderAdminItem {
    pub id: String,
    pub customer_name: String,
    pub phone: String,
    pub province: String,
    pub ward: String,
    pub address: String,
    pub delivery_note: Option<String>,
    pub items_json: String,
    pub status: String,
    pub created_at: String,
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/api/giot-viet/orders", post(create_order))
}

pub fn admin_router() -> Router<Arc<AppContext>> {
    Router::new().route("/api/admin/giot-viet/orders", get(list_orders))
}

pub async fn create_order(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<CreateOrderPayload>,
) -> ApiResult<CreateOrderResponse> {
    let order = validate_order(payload)?;
    let order_id = new_order_id();
    let received_at = Utc::now().to_rfc3339();
    let items_json = serde_json::to_string(&order.items)
        .map_err(|error| ApiError::internal(format!("serialize order items failed: {error}")))?;

    sqlx::query(
        r#"
        INSERT INTO giot_viet_physical_orders
            (id, customer_name, phone, province, ward, address, delivery_note, items_json, status, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'new', ?)
        "#,
    )
    .bind(&order_id)
    .bind(&order.customer_name)
    .bind(&order.phone)
    .bind(&order.province)
    .bind(&order.ward)
    .bind(&order.address)
    .bind(&order.delivery_note)
    .bind(&items_json)
    .bind(&received_at)
    .execute(&ctx.pool)
    .await
    .map_err(|error| ApiError::internal(format!("save Giot Viet order failed: {error}")))?;

    notify_admins(&ctx, &order_id, &received_at, &order).await;

    Ok(ok(CreateOrderResponse {
        order_id,
        received_at,
    }))
}

pub async fn list_orders(
    State(ctx): State<Arc<AppContext>>,
) -> ApiResult<Vec<PhysicalOrderAdminItem>> {
    let orders = sqlx::query_as::<_, PhysicalOrderAdminItem>(
        r#"
        SELECT id, customer_name, phone, province, ward, address, delivery_note, items_json, status, created_at
        FROM giot_viet_physical_orders
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&ctx.pool)
    .await
    .map_err(|error| ApiError::internal(format!("list Giot Viet orders failed: {error}")))?;

    Ok(ok(orders))
}

#[derive(Debug)]
struct ValidatedOrder {
    customer_name: String,
    phone: String,
    province: String,
    ward: String,
    address: String,
    delivery_note: Option<String>,
    items: Vec<StoredOrderItem>,
}

fn validate_order(payload: CreateOrderPayload) -> Result<ValidatedOrder, ApiError> {
    if !payload.website.trim().is_empty() {
        return Err(ApiError::validation("unable to submit order"));
    }

    let customer_name = required_text(&payload.customer_name, 100, "customer name")?;
    let phone = normalize_phone(&payload.phone)?;
    let province = required_text(&payload.province, 100, "province")?;
    let ward = required_text(&payload.ward, 100, "ward")?;
    let address = required_text(&payload.address, 250, "address")?;
    let delivery_note = optional_text(payload.delivery_note.as_deref(), 500, "delivery note")?;

    if payload.items.is_empty() || payload.items.len() > 3 {
        return Err(ApiError::validation("select at least one product"));
    }

    let mut items = Vec::with_capacity(payload.items.len());
    let mut total_quantity = 0_u16;
    for item in payload.items {
        if item.quantity == 0 || item.quantity > MAX_ITEM_QUANTITY {
            return Err(ApiError::validation("invalid item quantity"));
        }
        if items.iter().any(|existing: &StoredOrderItem| existing.sku == item.sku) {
            return Err(ApiError::validation("duplicate product"));
        }
        let Some(name) = product_name(&item.sku) else {
            return Err(ApiError::validation("invalid product"));
        };
        total_quantity = total_quantity.saturating_add(item.quantity);
        items.push(StoredOrderItem {
            sku: item.sku,
            name,
            quantity: item.quantity,
        });
    }

    if total_quantity > MAX_TOTAL_QUANTITY {
        return Err(ApiError::validation("too many items in one order"));
    }

    Ok(ValidatedOrder {
        customer_name,
        phone,
        province,
        ward,
        address,
        delivery_note,
        items,
    })
}

fn product_name(sku: &str) -> Option<&'static str> {
    match sku {
        "ca-com" => Some("Nước mắm Cá Cơm"),
        "mam-gung" => Some("Nước mắm Gừng"),
        "toi-ot" => Some("Nước mắm Tỏi Ớt"),
        _ => None,
    }
}

fn required_text(value: &str, max_length: usize, field: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_length {
        return Err(ApiError::validation(format!("{field} is invalid")));
    }
    Ok(value.to_string())
}

fn optional_text(
    value: Option<&str>,
    max_length: usize,
    field: &str,
) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > max_length {
        return Err(ApiError::validation(format!("{field} is invalid")));
    }
    Ok(Some(value.to_string()))
}

fn normalize_phone(value: &str) -> Result<String, ApiError> {
    let normalized: String = value
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == '+')
        .collect();
    let digits = normalized.trim_start_matches('+');
    let valid_length = (9..=15).contains(&digits.len());
    let valid_prefix = !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit());
    if !valid_length || !valid_prefix {
        return Err(ApiError::validation("phone is invalid"));
    }
    Ok(normalized)
}

fn new_order_id() -> String {
    let compact = Uuid::new_v4().simple().to_string();
    format!("GV-{}", compact[..10].to_ascii_uppercase())
}

async fn notify_admins(
    ctx: &AppContext,
    order_id: &str,
    received_at: &str,
    order: &ValidatedOrder,
) {
    let admin_ids = ctx.order_notification_admin_ids();
    if admin_ids.is_empty() {
        warn!(order_id = order_id, "Giot Viet order saved but no Telegram admin recipient is configured");
        return;
    }

    let items = order
        .items
        .iter()
        .map(|item| format!("- {} x{}", item.name, item.quantity))
        .collect::<Vec<_>>()
        .join("\n");
    let note = order
        .delivery_note
        .as_deref()
        .map(|value| format!("\nGhi chú: {value}"))
        .unwrap_or_default();
    let message = format!(
        "ĐƠN GIAO TẬN NƠI MỚI - GIỌT VIỆT\nMã đơn: {order_id}\nThời gian: {received_at}\n\nKhách: {}\nĐiện thoại: {}\nĐịa chỉ: {}, {}, {}{}\n\nSản phẩm:\n{items}",
        order.customer_name,
        order.phone,
        order.address,
        order.ward,
        order.province,
        note,
    );

    for admin_id in admin_ids {
        if let Err(error) = ctx.bot.send_message(ChatId(admin_id), &message).await {
            warn!(
                order_id = order_id,
                admin_id,
                error = %error,
                "send Giot Viet order Telegram notification failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(items: Vec<OrderItemPayload>) -> CreateOrderPayload {
        CreateOrderPayload {
            customer_name: "Nguyen Van An".to_string(),
            phone: "0901 234 567".to_string(),
            province: "TP. Ho Chi Minh".to_string(),
            ward: "Phuong 1".to_string(),
            address: "12 Nguyen Hue".to_string(),
            delivery_note: None,
            items,
            website: String::new(),
        }
    }

    #[test]
    fn accepts_known_products_and_normalizes_phone() {
        let order = validate_order(payload(vec![OrderItemPayload {
            sku: "ca-com".to_string(),
            quantity: 2,
        }]))
        .unwrap();

        assert_eq!(order.phone, "0901234567");
        assert_eq!(order.items[0].name, "Nước mắm Cá Cơm");
    }

    #[test]
    fn rejects_unknown_or_duplicate_products() {
        assert!(validate_order(payload(vec![OrderItemPayload {
            sku: "unknown".to_string(),
            quantity: 1,
        }]))
        .is_err());

        assert!(validate_order(payload(vec![
            OrderItemPayload {
                sku: "ca-com".to_string(),
                quantity: 1,
            },
            OrderItemPayload {
                sku: "ca-com".to_string(),
                quantity: 1,
            },
        ]))
        .is_err());
    }
}
