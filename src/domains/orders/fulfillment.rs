use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{error, warn};

use crate::app::AppContext;
use crate::domains::orders::admin_notify::notify_admins_order_paid;
use crate::domains::orders::api::{
    is_order_expired, parse_reserved_ids, product_delivery_type, send_product_file,
    take_account_stock_items,
};
use crate::domains::orders::models::{OrderStatus, OrderWithProduct};
use crate::domains::orders::repo;
use crate::domains::products::repo as products_repo;
use crate::domains::wallet::repo as wallet_repo;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentSource {
    BankWebhook { amount_vnd: i64 },
    BinancePay { prepay_id: String },
    Bep20 { tx_hash: String },
    AdminManual { admin_user_id: Option<i64> },
    Wallet,
    ClientApiWallet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FulfillOutcome {
    Delivered,
    AlreadyPaid,
    CreditedToWallet {
        balance_after: Option<i64>,
        reason: String,
    },
    Rejected {
        reason: String,
    },
}

pub async fn fulfill_paid_order(
    ctx: Arc<AppContext>,
    order_id: &str,
    payment_ref: &str,
    paid_at: DateTime<Utc>,
    source: PaymentSource,
) -> Result<FulfillOutcome> {
    let Some(mut order_with_product) = repo::get_order_with_product(&ctx.pool, order_id).await?
    else {
        return Ok(FulfillOutcome::Rejected {
            reason: "order not found".to_string(),
        });
    };

    if matches!(order_with_product.order.status, OrderStatus::Paid) {
        return Ok(FulfillOutcome::AlreadyPaid);
    }

    if !matches!(order_with_product.order.status, OrderStatus::Pending) {
        return credit_paid_order_to_wallet(
            &ctx,
            &order_with_product,
            order_with_product.order.status,
            paid_amount_vnd(&source, &order_with_product),
            "order is not pending",
        )
        .await;
    }

    if is_order_expired(&order_with_product.order.created_at) {
        return credit_paid_order_to_wallet(
            &ctx,
            &order_with_product,
            OrderStatus::Expired,
            paid_amount_vnd(&source, &order_with_product),
            "order expired",
        )
        .await;
    }

    let mut reserved_ids = order_with_product.order.reserved_item_ids.clone();

    let mut plugin_delivered_data = None;
    for plugin in ctx.plugins.iter() {
        match plugin
            .on_order_paid(
                ctx.clone(),
                &order_with_product.order,
                &order_with_product.product,
            )
            .await
        {
            Ok(Some(data)) => {
                plugin_delivered_data = Some(data);
                break;
            }
            Ok(None) => {}
            Err(err) => {
                error!(
                    "paid-order plugin {} failed for order {}: {err:#}",
                    plugin.name(),
                    order_with_product.order.id
                );
                if product_delivery_type(&order_with_product.product) == "external_api" {
                    return credit_paid_order_to_wallet(
                        &ctx,
                        &order_with_product,
                        OrderStatus::Cancel,
                        paid_amount_vnd(&source, &order_with_product),
                        "external API delivery failed",
                    )
                    .await;
                }
            }
        }
    }

    let mut tx = ctx.pool.begin().await?;
    let delivered_data = if let Some(data) = plugin_delivered_data {
        data
    } else if let Some(data) = &order_with_product.order.delivered_data {
        data.clone()
    } else {
        let delivery_result =
            if product_delivery_type(&order_with_product.product) == "uploaded_file" {
                match products_repo::take_product_items(
                    &mut tx,
                    order_with_product.order.product_id,
                    order_with_product.order.qty,
                )
                .await
                {
                    Ok(taken_items) => {
                        let ids: Vec<i64> = taken_items.iter().map(|item| item.id).collect();
                        let data = taken_items
                            .iter()
                            .map(|item| item.content.clone())
                            .collect::<Vec<_>>()
                            .join("\n");
                        (
                            data,
                            Some(
                                ids.iter()
                                    .map(|id| id.to_string())
                                    .collect::<Vec<_>>()
                                    .join(","),
                            ),
                        )
                    }
                    Err(err) => {
                        tx.rollback().await?;
                        warn!(
                            "order {} stock unavailable after payment: {err}",
                            order_with_product.order.id
                        );
                        return credit_paid_order_to_wallet(
                            &ctx,
                            &order_with_product,
                            OrderStatus::Cancel,
                            paid_amount_vnd(&source, &order_with_product),
                            "stock unavailable",
                        )
                        .await;
                    }
                }
        } else {
            match take_account_stock_items(
                &mut tx,
                order_with_product.order.product_id,
                order_with_product.order.qty,
                &order_with_product.order.id,
            )
            .await
            {
                Ok((data, ids)) => (data, ids),
                Err(err) => {
                    tx.rollback().await?;
                    warn!(
                        "order {} stock unavailable after payment: {err}",
                        order_with_product.order.id
                    );
                    return credit_paid_order_to_wallet(
                        &ctx,
                        &order_with_product,
                        OrderStatus::Cancel,
                        paid_amount_vnd(&source, &order_with_product),
                        "stock unavailable",
                    )
                    .await;
                }
            }
        };

        let (data, ids) = delivery_result;
        if data.is_empty() {
            tx.rollback().await?;
            return credit_paid_order_to_wallet(
                &ctx,
                &order_with_product,
                OrderStatus::Cancel,
                paid_amount_vnd(&source, &order_with_product),
                "no stock items available",
            )
            .await;
        }
        reserved_ids = ids;
        data
    };

    repo::mark_order_paid(
        &mut tx,
        &order_with_product.order.id,
        payment_ref,
        paid_at,
        Some(&delivered_data),
        reserved_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    order_with_product.order.status = OrderStatus::Paid;
    order_with_product.order.payment_tx_id = Some(payment_ref.to_string());
    order_with_product.order.paid_at = Some(paid_at.to_rfc3339());
    order_with_product.order.delivered_data = Some(delivered_data.clone());
    order_with_product.order.reserved_item_ids = reserved_ids;

    if let Err(err) = send_product_file(&ctx, &order_with_product, &delivered_data).await {
        error!("send product file failed for order {order_id}: {err}");
    }
    if let Err(err) =
        notify_admins_order_paid(&ctx, &order_with_product, payment_ref, paid_at, &source).await
    {
        error!("send paid-order admin notification failed for order {order_id}: {err}");
    }

    Ok(FulfillOutcome::Delivered)
}

fn paid_amount_vnd(source: &PaymentSource, order: &OrderWithProduct) -> i64 {
    match source {
        PaymentSource::BankWebhook { amount_vnd } => *amount_vnd,
        PaymentSource::AdminManual { .. }
        | PaymentSource::BinancePay { .. }
        | PaymentSource::Bep20 { .. }
        | PaymentSource::Wallet
        | PaymentSource::ClientApiWallet => order.order.amount,
    }
}

async fn credit_paid_order_to_wallet(
    ctx: &Arc<AppContext>,
    order: &OrderWithProduct,
    status: OrderStatus,
    amount: i64,
    reason: &str,
) -> Result<FulfillOutcome> {
    let mut tx = ctx.pool.begin().await?;
    if let Some(ids_str) = &order.order.reserved_item_ids {
        let ids = parse_reserved_ids(ids_str);
        if !ids.is_empty() {
            products_repo::return_product_items(&mut tx, order.order.product_id, &ids).await?;
        }
    }

    let note =
        format!("Payment received but order was not delivered ({reason}); credited to wallet");
    let balance_after = wallet_repo::credit_order_payment_to_wallet_once(
        &mut tx,
        order.order.user_id,
        amount,
        &order.order.id,
        Some(&note),
    )
    .await?;
    repo::update_order_status_with_data(&mut tx, &order.order.id, status, None, None).await?;
    tx.commit().await?;
    Ok(FulfillOutcome::CreditedToWallet {
        balance_after,
        reason: reason.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use teloxide::Bot;

    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::{Config, CryptoConfig};
    use crate::domains::orders::models::{Order, OrderReservationMode, OrderStatus};
    use crate::domains::orders::repo as orders_repo;
    use crate::domains::wallet::repo as wallet_repo;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_ctx(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        crate::app::AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: CryptoConfig::default(),
            },
            HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    async fn seed_product(pool: &SqlitePool) {
        sqlx::query("INSERT INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)")
            .bind(1_i64)
            .bind("Test product")
            .bind(50_000_i64)
            .bind(1_i64)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_order(pool: &SqlitePool, status: OrderStatus) -> Order {
        seed_product(pool).await;
        let mut order = Order::new(
            42,
            420,
            1,
            1,
            50_000,
            "DHFULFILL".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        order.status = status;
        orders_repo::insert_order(pool, &order).await.unwrap();
        order
    }

    async fn seed_no_reserve_order(pool: &SqlitePool) -> Order {
        seed_product(pool).await;
        let mut order = Order::new(
            42,
            420,
            1,
            1,
            50_000,
            format!("DHFULFILL{}", uuid::Uuid::new_v4().simple()),
            None,
            None,
            None,
            None,
            None,
        );
        order.reservation_mode = OrderReservationMode::NoReserve;
        orders_repo::insert_order(pool, &order).await.unwrap();
        order
    }

    #[tokio::test]
    async fn already_paid_order_returns_already_paid() {
        let pool = test_pool().await;
        let order = seed_order(&pool, OrderStatus::Paid).await;
        let ctx = test_ctx(pool);

        let outcome = fulfill_paid_order(
            ctx,
            &order.id,
            "tx-1",
            Utc::now(),
            PaymentSource::BankWebhook { amount_vnd: 50_000 },
        )
        .await
        .unwrap();

        assert_eq!(outcome, FulfillOutcome::AlreadyPaid);
    }

    #[tokio::test]
    async fn non_pending_paid_order_is_credited_once() {
        let pool = test_pool().await;
        let order = seed_order(&pool, OrderStatus::Expired).await;
        let ctx = test_ctx(pool.clone());

        let first = fulfill_paid_order(
            ctx.clone(),
            &order.id,
            "tx-1",
            Utc::now(),
            PaymentSource::BankWebhook { amount_vnd: 60_000 },
        )
        .await
        .unwrap();
        let second = fulfill_paid_order(
            ctx,
            &order.id,
            "tx-1",
            Utc::now(),
            PaymentSource::BankWebhook { amount_vnd: 60_000 },
        )
        .await
        .unwrap();

        assert_eq!(
            first,
            FulfillOutcome::CreditedToWallet {
                balance_after: Some(60_000),
                reason: "order is not pending".to_string()
            }
        );
        assert_eq!(
            second,
            FulfillOutcome::CreditedToWallet {
                balance_after: None,
                reason: "order is not pending".to_string()
            }
        );
        let wallet = wallet_repo::get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 60_000);
    }

    #[tokio::test]
    async fn no_reserve_paid_order_takes_available_stock_at_payment_time() {
        let pool = test_pool().await;
        let order = seed_no_reserve_order(&pool).await;
        crate::domains::products::repo::insert_product_items(
            &pool,
            order.product_id,
            &["user-at-payment|pass-at-payment".to_string()],
        )
        .await
        .unwrap();
        let ctx = test_ctx(pool.clone());

        let outcome = fulfill_paid_order(
            ctx,
            &order.id,
            "tx-no-reserve-1",
            Utc::now(),
            PaymentSource::BankWebhook { amount_vnd: 50_000 },
        )
        .await
        .unwrap();

        assert_eq!(outcome, FulfillOutcome::Delivered);
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Paid);
        assert_eq!(
            updated.delivered_data.as_deref(),
            Some("user-at-payment|pass-at-payment")
        );
        assert!(updated.reserved_item_ids.is_some());
        let remaining =
            crate::domains::products::repo::count_product_items(&pool, order.product_id)
                .await
                .unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn no_reserve_paid_order_without_stock_credits_wallet() {
        let pool = test_pool().await;
        let order = seed_no_reserve_order(&pool).await;
        let ctx = test_ctx(pool.clone());

        let outcome = fulfill_paid_order(
            ctx,
            &order.id,
            "tx-no-reserve-empty",
            Utc::now(),
            PaymentSource::BankWebhook { amount_vnd: 50_000 },
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            FulfillOutcome::CreditedToWallet {
                balance_after: Some(50_000),
                reason: "stock unavailable".to_string()
            }
        );
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Cancel);
        assert_eq!(updated.reserved_item_ids, None);
        let wallet = wallet_repo::get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 50_000);
    }
}
