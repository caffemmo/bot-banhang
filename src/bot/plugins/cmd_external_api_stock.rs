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
const DEFAULT_PRODUCT_DETAIL_URL: &str = "https://sumistore.me/api/tele-products/{product_id}";

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
    let api_id = required_config(
        ctx,
        "external_api_stock_api_id",
        "EXTERNAL_API_STOCK_API_ID",
    )?;
    let supplier_product_id = required_config(
        ctx,
        "external_api_stock_product_id",
        "EXTERNAL_API_STOCK_PRODUCT_ID",
    )?;
    let buy_url = optional_config(
        ctx,
        "external_api_stock_buy_url",
        "EXTERNAL_API_STOCK_BUY_URL",
        DEFAULT_BUY_URL,
    );
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
            api_response_detail(&raw)
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

pub async fn external_api_stock_count(ctx: &AppContext) -> Result<i64> {
    let api_id = required_config(
        ctx,
        "external_api_stock_api_id",
        "EXTERNAL_API_STOCK_API_ID",
    )?;
    let supplier_product_id = required_config(
        ctx,
        "external_api_stock_product_id",
        "EXTERNAL_API_STOCK_PRODUCT_ID",
    )?;
    let detail_url_template = optional_config(
        ctx,
        "external_api_stock_detail_url",
        "EXTERNAL_API_STOCK_DETAIL_URL",
        DEFAULT_PRODUCT_DETAIL_URL,
    );
    let detail_url = product_detail_url(&detail_url_template, &supplier_product_id);

    let response = Client::new()
        .get(detail_url)
        .header("X-Tele-API-ID", api_id)
        .send()
        .await
        .context("không gọi được API xem tồn kho ngoài")?;

    let status = response.status();
    let raw = response
        .text()
        .await
        .context("không đọc được response API xem tồn kho ngoài")?;
    if !status.is_success() {
        return Err(anyhow!(
            "API xem tồn kho ngoài trả HTTP {}: {}",
            status.as_u16(),
            api_response_detail(&raw)
        ));
    }

    let value: Value =
        serde_json::from_str(&raw).context("API xem tồn kho ngoài trả dữ liệu không phải JSON")?;
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(api_error_message(&value)));
    }

    json_i64_at_path(&value, &["product", "stock"])
        .or_else(|| json_i64_at_path(&value, &["stock"]))
        .ok_or_else(|| anyhow!("API xem tồn kho ngoài không trả product.stock"))
}

fn required_config(ctx: &AppContext, key: &str, env_key: &str) -> Result<String> {
    config_value(ctx, key, env_key, "")
        .trim()
        .to_string()
        .into_nonempty()
        .ok_or_else(|| anyhow!("chưa cấu hình {key} hoặc {env_key}"))
}

fn optional_config(ctx: &AppContext, key: &str, env_key: &str, default_value: &str) -> String {
    config_value(ctx, key, env_key, default_value)
        .trim()
        .to_string()
        .into_nonempty()
        .unwrap_or_else(|| default_value.to_string())
}

fn config_value(ctx: &AppContext, key: &str, env_key: &str, default_value: &str) -> String {
    let admin_value = ctx.get_text(key, "");
    if !admin_value.trim().is_empty() {
        return admin_value;
    }

    std::env::var(env_key).unwrap_or_else(|_| default_value.to_string())
}

fn product_detail_url(template: &str, product_id: &str) -> String {
    let trimmed = template.trim();
    if trimmed.contains("{product_id}") {
        return trimmed.replace("{product_id}", product_id);
    }
    if trimmed.trim_end_matches('/').ends_with(product_id) {
        return trimmed.to_string();
    }
    format!("{}/{}", trimmed.trim_end_matches('/'), product_id)
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

    collect_delivery_lines(value, &mut lines);

    lines.join("\n")
}

fn collect_delivery_lines(value: &Value, lines: &mut Vec<String>) {
    if let Some(line) = format_account_value(value) {
        if !lines.iter().any(|existing| existing == &line) {
            lines.push(line);
        }
        return;
    }

    match value {
        Value::Array(values) => {
            for item in values {
                collect_delivery_lines(item, lines);
            }
        }
        Value::Object(obj) => {
            for (key, item) in obj {
                if is_metadata_key(key) {
                    continue;
                }
                collect_delivery_lines(item, lines);
            }
        }
        _ => {}
    }
}

fn format_account_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        if !looks_like_delivery_text(text) {
            return None;
        }
        return Some(text.to_string());
    }
    let obj = value.as_object()?;
    if is_response_metadata_object(obj) {
        return None;
    }
    let mut fields = Vec::new();
    for key in [
        "account", "username", "email", "login", "password", "pass", "two_fa", "twofa", "2fa",
        "secret", "mail", "mail_password", "mail_pass", "recovery_mail", "code", "content",
        "cookie",
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
        None
    } else {
        Some(fields.join("|"))
    }
}

fn is_response_metadata_object(obj: &serde_json::Map<String, Value>) -> bool {
    let has_response_marker =
        obj.contains_key("success") || obj.contains_key("message") || obj.contains_key("error");
    let has_delivery_marker = [
        "account",
        "username",
        "email",
        "login",
        "password",
        "pass",
        "two_fa",
        "twofa",
        "2fa",
        "secret",
        "mail",
        "mail_password",
        "mail_pass",
        "recovery_mail",
        "content",
        "cookie",
    ]
    .iter()
    .any(|key| obj.contains_key(*key));

    has_response_marker && !has_delivery_marker
}

fn looks_like_delivery_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('|')
        || text.contains('\n')
        || text.contains('@')
        || lower.contains("password")
        || lower.contains("pass")
        || lower.contains("2fa")
}

fn is_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "success"
            | "code"
            | "message"
            | "error"
            | "owner"
            | "product"
            | "pricing"
            | "telegram_id"
            | "requested_product_id"
            | "quantity"
            | "stock"
            | "balance"
            | "balance_before"
            | "balance_after"
    )
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
    let code = json_string(value, "code");
    let message = json_string(value, "message").or_else(|| json_string(value, "error"));
    match (code, message) {
        (Some(code), Some(message)) if code != message => format!("{code}: {message}"),
        (Some(code), _) => code.to_string(),
        (_, Some(message)) => message.to_string(),
        _ => truncate_detail(&value.to_string()),
    }
}

fn json_i64_at_path(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_i64()
        .or_else(|| current.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| current.as_str()?.trim().parse::<i64>().ok())
}

fn api_response_detail(raw: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return api_error_message(&value);
    }

    truncate_detail(raw.lines().next().unwrap_or(raw))
}

fn truncate_detail(text: &str) -> String {
    text.chars().take(180).collect()
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

    #[test]
    fn external_delivery_reads_nested_accounts_without_status_code() {
        let value = json!({
            "success": true,
            "code": "TELE_PRODUCT_PURCHASED",
            "order_code": "API-TELE-ABC123",
            "data": {
                "items": [
                    {"login": "user@example.com", "pass": "secret", "code": "JBSWY3DPEHPK3PXP"}
                ]
            },
            "product": {"id": "SP-GEF55PBV", "name": "GPT PLUS"}
        });

        assert_eq!(
            format_external_delivery(&value),
            "order_code: API-TELE-ABC123\nuser@example.com|secret|JBSWY3DPEHPK3PXP"
        );
    }

    #[test]
    fn api_error_message_includes_code_and_message() {
        let value = json!({
            "success": false,
            "code": "INSUFFICIENT_BALANCE",
            "message": "Số dư không đủ"
        });

        assert_eq!(
            api_error_message(&value),
            "INSUFFICIENT_BALANCE: Số dư không đủ"
        );
    }
}
