use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{Duration, Utc};
use rand::{Rng, distributions::Alphanumeric};
use rust_decimal::Decimal;

use crate::app::AppContext;
use crate::core::crypto_amount::{calculate_usdt_base_amount, generate_bep20_unique_amount};
use crate::core::exchange_rate::{
    ConfiguredRateProvider, RateConfig, StaticRateProvider, get_usdt_rate_cached_or_static_fallback,
};
use crate::domains::crypto_pay::models::{CryptoPaymentMethod, CryptoPaymentRequest};
use crate::domains::crypto_pay::repo::{
    NewCryptoPayment, create_crypto_payment, find_crypto_payment_by_memo,
    find_pending_bep20_by_token_units, find_pending_crypto_payment_by_order,
};
use crate::domains::orders::models::OrderStatus;
use crate::domains::orders::repo as orders_repo;

pub fn format_bep20_amount(amount: Decimal) -> String {
    format!("{amount:.6}")
}

pub async fn create_or_reuse_bep20_payment(
    ctx: Arc<AppContext>,
    order_id: &str,
    user_id: i64,
    chat_id: i64,
) -> Result<CryptoPaymentRequest> {
    if !ctx.bep20_enabled() {
        return Err(anyhow!("BEP20 USDT payment is disabled"));
    }

    let order = orders_repo::get_order(&ctx.pool, order_id)
        .await?
        .ok_or_else(|| anyhow!("order not found"))?;
    if order.user_id != user_id || order.chat_id != chat_id {
        return Err(anyhow!("order does not belong to this user"));
    }
    if !matches!(order.status, OrderStatus::Pending) {
        return Err(anyhow!("order is not pending"));
    }

    if let Some(existing) = find_pending_crypto_payment_by_order(&ctx.pool, order_id).await?
        && existing.method == CryptoPaymentMethod::Bep20
    {
        return Ok(existing);
    }

    let rate = get_current_usdt_rate(&ctx).await?;
    let base = calculate_usdt_base_amount(order.amount, rate.buffered_rate_vnd_per_usdt)?;
    let amount = generate_bep20_unique_amount(base, |units| {
        let pool = ctx.pool.clone();
        async move {
            Ok(find_pending_bep20_by_token_units(&pool, &units)
                .await?
                .is_some())
        }
    })
    .await?;

    let memo = generate_crypto_memo(&ctx).await?;
    let wallet = ctx
        .bep20_merchant_wallet()
        .ok_or_else(|| anyhow!("BEP20 merchant wallet is missing"))?;
    let expires_at = Utc::now() + Duration::minutes(ctx.crypto_pay_ttl_minutes());

    create_crypto_payment(
        &ctx.pool,
        NewCryptoPayment {
            purpose: "order".to_string(),
            order_id: Some(order.id),
            wallet_topup_id: None,
            user_id,
            chat_id,
            method: CryptoPaymentMethod::Bep20,
            amount_vnd: order.amount,
            rate_vnd_per_usdt: rate.buffered_rate_vnd_per_usdt,
            amount_usdt_base: amount.base,
            amount_usdt_expected: amount.expected,
            amount_token_units: amount.token_units,
            memo,
            address: Some(wallet),
            binance_prepay_id: None,
            binance_checkout_url: None,
            binance_qrcode_link: None,
            binance_qr_content: None,
            binance_deeplink: None,
            binance_universal_url: None,
            expires_at: expires_at.to_rfc3339(),
        },
    )
    .await
}

pub async fn create_or_reuse_bep20_wallet_topup(
    ctx: Arc<AppContext>,
    user_id: i64,
    chat_id: i64,
    amount_vnd: i64,
) -> Result<CryptoPaymentRequest> {
    if !ctx.bep20_enabled() {
        return Err(anyhow!("BEP20 USDT payment is disabled"));
    }
    if !(10_000..=100_000_000).contains(&amount_vnd) {
        return Err(anyhow!("invalid top-up amount"));
    }

    let rate = get_current_usdt_rate(&ctx).await?;
    let base = calculate_usdt_base_amount(amount_vnd, rate.buffered_rate_vnd_per_usdt)?;
    let amount = generate_bep20_unique_amount(base, |units| {
        let pool = ctx.pool.clone();
        async move {
            Ok(find_pending_bep20_by_token_units(&pool, &units)
                .await?
                .is_some())
        }
    })
    .await?;

    let memo = generate_crypto_memo(&ctx).await?;
    let wallet = ctx
        .bep20_merchant_wallet()
        .ok_or_else(|| anyhow!("BEP20 merchant wallet is missing"))?;
    let expires_at = Utc::now() + Duration::minutes(ctx.crypto_pay_ttl_minutes());

    create_crypto_payment(
        &ctx.pool,
        NewCryptoPayment {
            purpose: "wallet_topup".to_string(),
            order_id: None,
            wallet_topup_id: None,
            user_id,
            chat_id,
            method: CryptoPaymentMethod::Bep20,
            amount_vnd,
            rate_vnd_per_usdt: rate.buffered_rate_vnd_per_usdt,
            amount_usdt_base: amount.base,
            amount_usdt_expected: amount.expected,
            amount_token_units: amount.token_units,
            memo,
            address: Some(wallet),
            binance_prepay_id: None,
            binance_checkout_url: None,
            binance_qrcode_link: None,
            binance_qr_content: None,
            binance_deeplink: None,
            binance_universal_url: None,
            expires_at: expires_at.to_rfc3339(),
        },
    )
    .await
}

async fn get_current_usdt_rate(ctx: &AppContext) -> Result<crate::core::exchange_rate::UsdtRate> {
    let fallback = StaticRateProvider {
        rate_vnd_per_usdt: ctx.usd_vnd_fallback_rate(),
        source: "runtime_fallback".to_string(),
    };
    let provider = ctx
        .usdt_rate_custom_url()
        .map(|url| ConfiguredRateProvider::HttpJson { url })
        .unwrap_or_else(|| ConfiguredRateProvider::Static(fallback.clone()));

    get_usdt_rate_cached_or_static_fallback(
        &provider,
        fallback,
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

async fn generate_crypto_memo(ctx: &AppContext) -> Result<String> {
    for _ in 0..10 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_alphanumeric())
            .map(char::from)
            .take(10)
            .collect::<String>()
            .to_uppercase();
        let memo = format!("CPR{suffix}");
        if find_crypto_payment_by_memo(&ctx.pool, &memo)
            .await?
            .is_none()
        {
            return Ok(memo);
        }
    }
    Err(anyhow!("Không tạo được mã thanh toán USDT unique"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use teloxide::Bot;

    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use crate::domains::crypto_pay::models::CryptoPaymentMethod;
    use crate::domains::orders::models::Order;
    use crate::domains::orders::repo as orders_repo;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_config() -> Config {
        let mut env = HashMap::from([
            ("TELOXIDE_TOKEN".to_string(), "test-token".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "test-admin-jwt-secret-at-least-32-chars".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup-code".to_string()),
            (
                "BEP20_MERCHANT_WALLET".to_string(),
                "0x0000000000000000000000000000000000000001".to_string(),
            ),
            ("BSCSCAN_API_KEY".to_string(), "bsc-key".to_string()),
        ]);
        env.insert("USD_VND_FALLBACK_RATE".to_string(), "25000".to_string());
        env.insert("USDT_RATE_BUFFER_PERCENT".to_string(), "1".to_string());
        Config::from_env_map(&env).unwrap()
    }

    fn test_ctx(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        crate::app::AppContext::new(
            Bot::new("test-token"),
            pool,
            test_config(),
            HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    async fn seed_order(pool: &SqlitePool) -> Order {
        sqlx::query("INSERT INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)")
            .bind(1_i64)
            .bind("BEP20 product")
            .bind(50_000_i64)
            .bind(1_i64)
            .execute(pool)
            .await
            .unwrap();
        let order = Order::new(
            42,
            420,
            1,
            1,
            50_000,
            "DHBEP2001".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        orders_repo::insert_order(pool, &order).await.unwrap();
        order
    }

    #[test]
    fn bep20_amount_display_uses_six_decimals() {
        assert_eq!(
            format_bep20_amount(rust_decimal_macros::dec!(2.5)),
            "2.500000"
        );
    }

    #[tokio::test]
    async fn create_or_reuse_bep20_payment_reuses_active_request() {
        let pool = test_pool().await;
        let order = seed_order(&pool).await;
        let ctx = test_ctx(pool);

        let first = create_or_reuse_bep20_payment(ctx.clone(), &order.id, 42, 420)
            .await
            .unwrap();
        let second = create_or_reuse_bep20_payment(ctx, &order.id, 42, 420)
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(first.method, CryptoPaymentMethod::Bep20);
        assert_eq!(first.amount_vnd, 50_000);
        assert_eq!(first.amount_usdt_base.to_string(), "1.99");
        assert_eq!(format_bep20_amount(first.amount_usdt_expected).len(), 8);
    }
}
