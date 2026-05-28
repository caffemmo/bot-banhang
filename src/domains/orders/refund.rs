use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::app::AppContext;
use crate::domains::orders::models::{OrderStatus, OrderWithProduct};
use crate::domains::orders::repo as orders_repo;
use crate::domains::wallet::repo as wallet_repo;

#[derive(Debug, Clone)]
pub struct RefundOutcome {
    pub order: OrderWithProduct,
    pub balance_after: Option<i64>,
    pub username: String,
}

pub async fn refund_paid_order_to_wallet(
    ctx: &Arc<AppContext>,
    order_id: &str,
    admin_user_id: i64,
    username: &str,
) -> Result<RefundOutcome> {
    let Some(order) = orders_repo::get_order_with_product(&ctx.pool, order_id).await? else {
        return Err(anyhow!("order not found"));
    };

    if !matches!(
        order.order.status,
        OrderStatus::Paid | OrderStatus::Refunded
    ) {
        return Err(anyhow!("only paid orders can be refunded"));
    }

    let note = format!("Refund paid order by admin {admin_user_id}; user {username}");
    let mut tx = ctx.pool.begin().await?;
    let balance_after = wallet_repo::credit_order_payment_to_wallet_once(
        &mut tx,
        order.order.user_id,
        order.order.amount,
        &order.order.id,
        Some(&note),
    )
    .await?;
    orders_repo::update_order_status(&mut tx, &order.order.id, OrderStatus::Refunded).await?;
    tx.commit().await?;

    let mut updated = order;
    updated.order.status = OrderStatus::Refunded;

    Ok(RefundOutcome {
        order: updated,
        balance_after,
        username: username.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use teloxide::Bot;

    use crate::app::AppContext;
    use crate::bot::texts::BotTexts;
    use crate::config::{Config, CryptoConfig};
    use crate::domains::orders::models::{Order, OrderStatus};
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

    fn test_ctx(pool: SqlitePool) -> std::sync::Arc<AppContext> {
        AppContext::new(
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

    async fn seed_paid_order(pool: &SqlitePool) -> Order {
        sqlx::query("INSERT INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)")
            .bind(1_i64)
            .bind("Test product")
            .bind(50_000_i64)
            .bind(1_i64)
            .execute(pool)
            .await
            .unwrap();

        let mut order = Order::new(
            42,
            420,
            1,
            1,
            50_000,
            "DHREFUND".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        order.status = OrderStatus::Paid;
        orders_repo::insert_order(pool, &order).await.unwrap();
        order
    }

    #[tokio::test]
    async fn paid_order_refund_credits_wallet_once_and_marks_refunded() {
        let pool = test_pool().await;
        let order = seed_paid_order(&pool).await;
        let ctx = test_ctx(pool.clone());

        let first = super::refund_paid_order_to_wallet(&ctx, &order.id, 7, "@alice")
            .await
            .unwrap();
        let second = super::refund_paid_order_to_wallet(&ctx, &order.id, 7, "@alice")
            .await
            .unwrap();

        assert_eq!(first.balance_after, Some(50_000));
        assert_eq!(first.username, "@alice");
        assert_eq!(second.balance_after, None);
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Refunded);
        let wallet = wallet_repo::get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 50_000);
    }
}
