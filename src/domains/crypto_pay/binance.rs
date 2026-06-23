#![allow(dead_code)]

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::{Rng, distributions::Alphanumeric};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha512;

use crate::app::AppContext;
use crate::config::BinancePayEnv;
use crate::core::crypto_amount::{calculate_usdt_base_amount, decimal_to_token_units};
use crate::core::exchange_rate::{
    ConfiguredRateProvider, RateConfig, StaticRateProvider, get_usdt_rate_cached,
};
use crate::domains::crypto_pay::models::{CryptoPaymentMethod, CryptoPaymentRequest};
use crate::domains::crypto_pay::repo::{
    NewCryptoPayment, create_crypto_payment, find_crypto_payment_by_memo,
    find_pending_crypto_payment_by_order,
};
use crate::domains::orders::models::OrderStatus;
use crate::domains::orders::repo as orders_repo;

type HmacSha512 = Hmac<Sha512>;

pub fn sign_payload(timestamp: &str, nonce: &str, body: &str, secret: &str) -> String {
    let payload = format!("{timestamp}\n{nonce}\n{body}\n");
    let mut mac =
        HmacSha512::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(payload.as_bytes());
    hex::encode_upper(mac.finalize().into_bytes())
}

#[derive(Debug, Clone)]
pub struct CreateOrderRequest {
    pub merchant_trade_no: String,
    pub order_amount: Decimal,
    pub product_name: String,
    pub product_detail: String,
    pub order_expire_time_ms: i64,
    pub webhook_url: Option<String>,
    pub return_url: Option<String>,
    pub cancel_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CreateOrderResult {
    #[serde(rename = "prepayId")]
    pub prepay_id: String,
    #[serde(rename = "qrcodeLink")]
    pub qrcode_link: String,
    #[serde(rename = "qrContent")]
    pub qr_content: String,
    #[serde(rename = "checkoutUrl")]
    pub checkout_url: String,
    pub deeplink: String,
    #[serde(rename = "universalUrl")]
    pub universal_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryOrderResult {
    #[serde(rename = "prepayId")]
    pub prepay_id: String,
    #[serde(rename = "transactionId")]
    pub transaction_id: Option<String>,
    #[serde(rename = "merchantTradeNo")]
    pub merchant_trade_no: String,
    pub status: String,
    pub currency: String,
    #[serde(rename = "orderAmount")]
    pub order_amount: String,
    #[serde(rename = "openUserId")]
    pub open_user_id: Option<String>,
    #[serde(rename = "transactTime")]
    pub transact_time: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloseOrderResult {
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct BinanceApiResponse<T> {
    status: String,
    code: String,
    data: Option<T>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

#[derive(Clone)]
pub struct BinancePayClient {
    api_key: String,
    secret: String,
    cert_sn: String,
    http: reqwest::Client,
    base_url: String,
}

impl BinancePayClient {
    pub fn from_context(ctx: &AppContext) -> Result<Self> {
        let binance = &ctx.config.crypto.binance;
        let base_url = match binance.env {
            BinancePayEnv::Sandbox | BinancePayEnv::Production => "https://bpay.binanceapi.com",
        };
        Ok(Self {
            api_key: ctx
                .binance_pay_api_key()
                .ok_or_else(|| anyhow!("BINANCE_PAY_API_KEY is missing"))?,
            secret: ctx
                .binance_pay_secret()
                .ok_or_else(|| anyhow!("BINANCE_PAY_SECRET is missing"))?,
            cert_sn: ctx
                .binance_pay_cert_sn()
                .ok_or_else(|| anyhow!("BINANCE_PAY_CERT_SN is missing"))?,
            http: reqwest::Client::new(),
            base_url: base_url.to_string(),
        })
    }

    pub async fn create_order(&self, req: &CreateOrderRequest) -> Result<CreateOrderResult> {
        let body = build_create_order_body(req)?;
        self.post_signed("/binancepay/openapi/v2/order", body).await
    }

    pub async fn query_order(
        &self,
        merchant_trade_no: &str,
        prepay_id: Option<&str>,
    ) -> Result<QueryOrderResult> {
        let body = if let Some(prepay_id) = prepay_id {
            json!({ "prepayId": prepay_id })
        } else {
            json!({ "merchantTradeNo": merchant_trade_no })
        };
        self.post_signed("/binancepay/openapi/v2/order/query", body)
            .await
    }

    pub async fn close_order(&self, merchant_trade_no: &str) -> Result<CloseOrderResult> {
        self.post_signed(
            "/binancepay/openapi/order/close",
            json!({ "merchantTradeNo": merchant_trade_no }),
        )
        .await
    }

    async fn post_signed<T>(&self, path: &str, body: Value) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let body = serde_json::to_string(&body)?;
        let timestamp = Utc::now().timestamp_millis().to_string();
        let nonce = generate_nonce();
        let signature = sign_payload(&timestamp, &nonce, &body, &self.secret);
        let response = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .header("Content-Type", "application/json")
            .header("BinancePay-Timestamp", timestamp)
            .header("BinancePay-Nonce", nonce)
            .header("BinancePay-Certificate-SN", &self.cert_sn)
            .header("BinancePay-Signature", signature)
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json::<BinanceApiResponse<T>>()
            .await?;
        if response.status == "SUCCESS" && response.code == "000000" {
            response
                .data
                .ok_or_else(|| anyhow!("Binance Pay success response missing data"))
        } else {
            Err(anyhow!(
                "Binance Pay API failed {}: {}",
                response.code,
                response
                    .error_message
                    .unwrap_or_else(|| "unknown error".to_string())
            ))
        }
    }
}

pub fn build_create_order_body(req: &CreateOrderRequest) -> Result<Value> {
    let mut body = json!({
        "env": { "terminalType": "WEB" },
        "merchantTradeNo": req.merchant_trade_no,
        "orderAmount": decimal_to_json_number(req.order_amount)?,
        "currency": "USDT",
        "goods": {
            "goodsType": "02",
            "goodsCategory": "Z000",
            "referenceGoodsId": req.merchant_trade_no,
            "goodsName": sanitize_goods_name(&req.product_name),
            "goodsDetail": sanitize_goods_name(&req.product_detail),
        },
        "orderExpireTime": req.order_expire_time_ms,
        "supportPayCurrency": "USDT"
    });
    if let Some(webhook_url) = &req.webhook_url {
        body["webhookUrl"] = Value::String(webhook_url.clone());
    }
    if let Some(return_url) = &req.return_url {
        body["returnUrl"] = Value::String(return_url.clone());
    }
    if let Some(cancel_url) = &req.cancel_url {
        body["cancelUrl"] = Value::String(cancel_url.clone());
    }
    Ok(body)
}

pub async fn create_or_reuse_binance_payment(
    ctx: Arc<AppContext>,
    order_id: &str,
    user_id: i64,
    chat_id: i64,
) -> Result<CryptoPaymentRequest> {
    if !ctx.binance_pay_enabled() {
        return Err(anyhow!("Binance Pay is disabled"));
    }

    let order_with_product = orders_repo::get_order_with_product(&ctx.pool, order_id)
        .await?
        .ok_or_else(|| anyhow!("order not found"))?;
    let order = &order_with_product.order;
    if order.user_id != user_id || order.chat_id != chat_id {
        return Err(anyhow!("order does not belong to this user"));
    }
    if !matches!(order.status, OrderStatus::Pending) {
        return Err(anyhow!("order is not pending"));
    }

    if let Some(existing) = find_pending_crypto_payment_by_order(&ctx.pool, order_id).await?
        && existing.method == CryptoPaymentMethod::BinancePay
    {
        return Ok(existing);
    }

    let rate = get_current_usdt_rate(&ctx).await?;
    let base = calculate_usdt_base_amount(order.amount, rate.buffered_rate_vnd_per_usdt)?;
    let token_units = decimal_to_token_units(base, 18)?;
    let memo = generate_binance_pay_note_code(&ctx).await?;
    let expires_at = Utc::now() + Duration::minutes(ctx.crypto_pay_ttl_minutes());

    let payment = create_crypto_payment(
        &ctx.pool,
        NewCryptoPayment {
            purpose: "order".to_string(),
            order_id: Some(order.id.clone()),
            wallet_topup_id: None,
            user_id,
            chat_id,
            method: CryptoPaymentMethod::BinancePay,
            amount_vnd: order.amount,
            rate_vnd_per_usdt: rate.buffered_rate_vnd_per_usdt,
            amount_usdt_base: base,
            amount_usdt_expected: base,
            amount_token_units: token_units,
            memo: memo.clone(),
            address: None,
            binance_prepay_id: None,
            binance_checkout_url: None,
            binance_qrcode_link: None,
            binance_qr_content: None,
            binance_deeplink: None,
            binance_universal_url: None,
            expires_at: expires_at.to_rfc3339(),
        },
    )
    .await?;
    Ok(payment)
}

pub async fn create_or_reuse_binance_wallet_topup(
    ctx: Arc<AppContext>,
    user_id: i64,
    chat_id: i64,
    amount_vnd: i64,
) -> Result<CryptoPaymentRequest> {
    if !ctx.binance_pay_enabled() {
        return Err(anyhow!("Binance Pay is disabled"));
    }
    if !(10_000..=100_000_000).contains(&amount_vnd) {
        return Err(anyhow!("invalid top-up amount"));
    }

    let rate = get_current_usdt_rate(&ctx).await?;
    let base = calculate_usdt_base_amount(amount_vnd, rate.buffered_rate_vnd_per_usdt)?;
    let token_units = decimal_to_token_units(base, 18)?;
    let memo = generate_binance_pay_note_code(&ctx).await?;
    let expires_at = Utc::now() + Duration::minutes(ctx.crypto_pay_ttl_minutes());

    let payment = create_crypto_payment(
        &ctx.pool,
        NewCryptoPayment {
            purpose: "wallet_topup".to_string(),
            order_id: None,
            wallet_topup_id: None,
            user_id,
            chat_id,
            method: CryptoPaymentMethod::BinancePay,
            amount_vnd,
            rate_vnd_per_usdt: rate.buffered_rate_vnd_per_usdt,
            amount_usdt_base: base,
            amount_usdt_expected: base,
            amount_token_units: token_units,
            memo: memo.clone(),
            address: None,
            binance_prepay_id: None,
            binance_checkout_url: None,
            binance_qrcode_link: None,
            binance_qr_content: None,
            binance_deeplink: None,
            binance_universal_url: None,
            expires_at: expires_at.to_rfc3339(),
        },
    )
    .await?;
    Ok(payment)
}

async fn get_current_usdt_rate(ctx: &AppContext) -> Result<crate::core::exchange_rate::UsdtRate> {
    let provider = if let Some(url) = ctx.usdt_rate_custom_url() {
        ConfiguredRateProvider::HttpJson { url }
    } else {
        ConfiguredRateProvider::Static(StaticRateProvider {
            rate_vnd_per_usdt: ctx.usd_vnd_fallback_rate(),
            source: "runtime_fallback".to_string(),
        })
    };
    get_usdt_rate_cached(
        &provider,
        &ctx.usdt_rate_cache,
        RateConfig {
            buffer_percent: ctx.usdt_rate_buffer_percent(),
            cache_seconds: ctx.usdt_rate_cache_seconds(),
            stale_seconds: ctx.usdt_rate_stale_seconds(),
        },
        Utc::now(),
    )
    .await
}

async fn generate_binance_merchant_trade_no(ctx: &AppContext) -> Result<String> {
    for _ in 0..10 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_alphanumeric())
            .map(char::from)
            .take(20)
            .collect::<String>()
            .to_uppercase();
        let memo = format!("BP{suffix}");
        if find_crypto_payment_by_memo(&ctx.pool, &memo)
            .await?
            .is_none()
        {
            return Ok(memo);
        }
    }
    Err(anyhow!("Không tạo được mã Binance Pay unique"))
}

pub async fn generate_binance_pay_note_code(ctx: &AppContext) -> Result<String> {
    let prefix = ctx.binance_pay_note_prefix();
    let digits = ctx.binance_pay_note_digits();
    for _ in 0..20 {
        let memo = format!("{}{}", prefix, random_digit_string(digits));
        if find_crypto_payment_by_memo(&ctx.pool, &memo)
            .await?
            .is_none()
        {
            return Ok(memo);
        }
    }
    for _ in 0..20 {
        let memo = format!("{}{}", prefix, random_digit_string(8));
        if find_crypto_payment_by_memo(&ctx.pool, &memo)
            .await?
            .is_none()
        {
            return Ok(memo);
        }
    }
    Err(anyhow!("Không tạo được mã Binance Pay unique"))
}

fn random_digit_string(digits: u8) -> String {
    let mut rng = rand::thread_rng();
    (0..digits)
        .map(|_| char::from(b'0' + rng.gen_range(0..10)))
        .collect()
}

fn generate_nonce() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .filter(|c| c.is_ascii_alphanumeric())
        .map(char::from)
        .take(32)
        .collect()
}

fn decimal_to_json_number(decimal: Decimal) -> Result<Value> {
    let number = serde_json::Number::from_str(&decimal.normalize().to_string())?;
    Ok(Value::Number(number))
}

fn sanitize_goods_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|ch| ch.is_ascii() && !matches!(ch, '"' | '\\'))
        .collect::<String>();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "Digital goods".to_string()
    } else {
        trimmed.chars().take(256).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn signs_payload_with_uppercase_hmac_sha512() {
        let body = r#"{"merchantTradeNo":"ORDER123","orderAmount":"1.23","currency":"USDT"}"#;

        let signature = sign_payload(
            "1656088792123",
            "nonce12345678901234567890123456",
            body,
            "test-secret",
        );

        assert_eq!(
            signature,
            "A2E966319A0DF660E5BBE594FA3DED724B60A2559365968FE56470C28BC1696AF96A121FEA59892FE22DD8D082D4575B17E877C047763DB275C4E7DFA786A797"
        );
    }

    #[test]
    fn builds_create_order_v2_body() {
        let body = build_create_order_body(&CreateOrderRequest {
            merchant_trade_no: "BPABC123".to_string(),
            order_amount: rust_decimal_macros::dec!(1.23),
            product_name: "Digital \"Product\" ✅".to_string(),
            product_detail: "Order #1".to_string(),
            order_expire_time_ms: 1_716_000_000_000,
            webhook_url: Some("https://example.com/webhook".to_string()),
            return_url: None,
            cancel_url: None,
        })
        .unwrap();

        assert_eq!(body["merchantTradeNo"], "BPABC123");
        assert_eq!(body["currency"], "USDT");
        assert_eq!(body["orderAmount"], json!(1.23));
        assert_eq!(body["goods"]["goodsType"], "02");
        assert_eq!(body["goods"]["goodsName"], "Digital Product");
        assert_eq!(body["webhookUrl"], "https://example.com/webhook");
    }

    #[tokio::test]
    async fn binance_note_code_uses_configured_prefix_and_digits() {
        let ctx = test_ctx().await;

        let note = generate_binance_pay_note_code(&ctx).await.unwrap();

        assert_eq!(note.len(), 8);
        assert!(note.starts_with("VI"));
        assert!(note[2..].bytes().all(|b| b.is_ascii_digit()));
    }

    #[tokio::test]
    async fn wallet_topup_creates_local_note_payment_without_merchant_checkout() {
        let ctx = test_ctx().await;

        let payment = create_or_reuse_binance_wallet_topup(ctx, 42, 420, 26_000)
            .await
            .unwrap();

        assert_eq!(payment.purpose, "wallet_topup");
        assert_eq!(payment.method, CryptoPaymentMethod::BinancePay);
        assert!(payment.memo.starts_with("VI"));
        assert_eq!(payment.memo.len(), 8);
        assert_eq!(payment.binance_checkout_url, None);
        assert_eq!(payment.binance_prepay_id, None);
        assert_eq!(payment.amount_usdt_expected, rust_decimal_macros::dec!(1));
    }

    async fn test_ctx() -> Arc<AppContext> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut env = std::collections::HashMap::from([
            ("TELOXIDE_TOKEN".to_string(), "test-token".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "test-admin-jwt-secret-at-least-32-chars".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup-code".to_string()),
            ("BINANCE_PAY_NOTE_ENABLED".to_string(), "1".to_string()),
            ("BINANCE_PAY_API_KEY".to_string(), "api-key".to_string()),
            (
                "BINANCE_PAY_API_SECRET".to_string(),
                "api-secret".to_string(),
            ),
            (
                "BINANCE_PAY_RECEIVER_PAY_ID".to_string(),
                "209378262".to_string(),
            ),
            (
                "BINANCE_PAY_RECEIVER_NAME".to_string(),
                "Receiver".to_string(),
            ),
            ("USD_VND_FALLBACK_RATE".to_string(), "26000".to_string()),
            ("USDT_RATE_BUFFER_PERCENT".to_string(), "1".to_string()),
        ]);
        let config = Config::from_env_map(&env).unwrap();
        env.clear();
        AppContext::new(
            teloxide::Bot::new("test-token"),
            pool,
            config,
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }
}
