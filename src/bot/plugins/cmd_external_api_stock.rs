use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::domains::orders::api as orders_api;
use crate::domains::orders::models::Order;
use crate::domains::products::models::Product;

const DEFAULT_BUY_URL: &str = "https://sumistore.me/api/tele-product/buy";

type HmacSha256 = Hmac<Sha256>;

pub struct ExternalApiStockPlugin;

#[async_trait::async_trait]
impl AppPlugin for ExternalApiStockPlugin {
    fn name(&self) -> &'static str {
        "ExternalApiStock"
    }

    async fn on_order_paid(
        &self,
        ctx: Arc<AppContext>,
        order: &Order,
        product: &Product,
    ) -> Result<Option<String>, anyhow::Error> {
        if orders_api::product_delivery_type(product) != "external_api" {
            return Ok(None);
        }

        match buy_external_stock(&ctx, order.qty).await {
            Ok(delivered_data) => Ok(Some(delivered_data)),
            Err(err) => {
                tracing::error!(
                    "external API stock buy failed for order {} product {}: {err:#}",
                    order.id,
                    product.id
                );
                Err(err)
            }
        }
    }
}

async fn buy_external_stock(ctx: &AppContext, quantity: i64) -> Result<String> {
    let api_id = required_config(ctx, "external_api_stock_api_id")?;
    let supplier_product_id = required_config(ctx, "external_api_stock_product_id")?;
    let buy_url = ctx.get_text("external_api_stock_buy_url", DEFAULT_BUY_URL);
    let quantity = quantity.max(1);
    let body = json!({
        "id": supplier_product_id,
        "quantity": quantity,
    })
    .to_string();
    let timestamp = unix_timestamp_seconds()?;
    let nonce = Uuid::new_v4().simple().to_string();
    let signature = hmac_signature(&api_id, timestamp, &nonce, &body)?;

    let response = Client::new()
        .post(buy_url)
        .header("Content-Type", "application/json")
        .header("X-Tele-API-ID", api_id)
        .header("X-Timestamp", timestamp.to_string())
        .header("X-Nonce", nonce)
        .header("X-Signature", signature)
        .body(body)
        .send()
        .await
        .context("không gọi được API mua hàng ngoài")?;

    let status = response.status();
    let raw = response
        .text()
        .await
        .context("không đọc được response API mua hàng ngoài")?;
    if !status.is_success() {
        return Err(anyhow!(
            "API mua hàng ngoài trả HTTP {}: {}",
            status.as_u16(),
            friendly_api_detail(&raw)
        ));
    }

    let value: Value =
        serde_json::from_str(&raw).context("API mua hàng ngoài trả dữ liệu không phải JSON")?;
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(api_error_message(&value)));
    }

    let delivered = format_external_delivery(&value);
    if delivered.trim().is_empty() {
        return Err(anyhow!("API mua hàng thành công nhưng không trả account"));
    }
    Ok(delivered)
}

fn required_config(ctx: &AppContext, key: &str) -> Result<String> {
    ctx.get_text(key, "")
        .trim()
        .to_string()
        .into_nonempty()
        .ok_or_else(|| anyhow!("chưa cấu hình {key}"))
}

fn hmac_signature(secret: &str, timestamp: i64, nonce: &str, body: &str) -> Result<String> {
    let payload = format!("{timestamp}|{nonce}|{body}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .context("không tạo được HMAC cho API mua hàng ngoài")?;
    mac.update(payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn unix_timestamp_seconds() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("đồng hồ hệ thống đang trước Unix epoch")?
        .as_secs() as i64)
}

fn format_external_delivery(value: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(order_code) = json_string(value, "order_code")
        .or_else(|| json_string(value, "orderCode"))
        .or_else(|| json_string(value, "id"))
    {
        lines.push(format!("order_code: {order_code}"));
    }

    if let Some(accounts) = value.get("accounts").and_then(Value::as_array) {
        for account in accounts {
            if let Some(line) = format_account_value(account) {
                lines.push(line);
            }
        }
    }

    if lines.len() <= 1
        && let Some(account) = value.get("account")
        && let Some(line) = format_account_value(account)
    {
        lines.push(line);
    }

    if lines.len() <= 1
        && let Some(data) = value.get("data")
        && let Some(line) = format_account_value(data)
    {
        lines.push(line);
    }

    lines.join("\n")
}

fn format_account_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(text.to_string());
    }
    let obj = value.as_object()?;
    let mut fields = Vec::new();
    for key in [
        "account", "username", "email", "login", "password", "pass", "two_fa", "twofa", "2fa",
        "mail", "mail_password", "code", "content",
    ] {
        if let Some(text) = obj
            .get(key)
            .and_then(json_value_to_string)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            fields.push(text);
        }
    }

    if fields.is_empty() {
        let text = serde_json::to_string(value).ok()?;
        if text == "{}" { None } else { Some(text) }
    } else {
        Some(fields.join("|"))
    }
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn json_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn api_error_message(value: &Value) -> String {
    for key in ["message", "error", "code"] {
        if let Some(text) = json_string(value, key) {
            return text.to_string();
        }
    }
    friendly_api_detail(&value.to_string())
}

fn friendly_api_detail(raw: &str) -> String {
    raw.lines()
        .next()
        .unwrap_or(raw)
        .chars()
        .take(180)
        .collect()
}

trait NonEmptyString {
    fn into_nonempty(self) -> Option<String>;
}

impl NonEmptyString for String {
    fn into_nonempty(self) -> Option<String> {
        if self.is_empty() { None } else { Some(self) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_signature_matches_documented_payload() {
        let signature = hmac_signature(
            "TAPI-JGL4Z3OXYWOGF3HBBCCE",
            1700000000,
            "nonce",
            r#"{"id":"SP-GEF55PBV","quantity":1}"#,
        )
        .unwrap();

        assert_eq!(
            signature,
            "bf093215a2612e2e4d363e212eb3cf88704d06583d7c1875fc2d489620e8bd98"
        );
    }

    #[test]
    fn external_delivery_formats_account_objects() {
        let value = json!({
            "success": true,
            "order_code": "API-TELE-ABC123",
            "accounts": [
                {"email": "a@example.com", "password": "pass", "twofa": "ABCDEF"}
            ]
        });

        assert_eq!(
            format_external_delivery(&value),
            "order_code: API-TELE-ABC123\na@example.com|pass|ABCDEF"
        );
    }
}
