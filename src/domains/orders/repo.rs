use crate::domains::orders::models::{Order, OrderReservationMode, OrderStatus, OrderWithProduct};
use crate::domains::products::models::Product;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, QueryBuilder, Sqlite, SqlitePool, Transaction};

use super::models::OrderSupportRequest;

#[derive(Debug, FromRow)]
#[allow(dead_code)]
pub struct OrderJoinRow {
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
    pub p_id: i64,
    pub p_name: String,
    pub p_price: i64,
    pub p_is_active: Option<i64>,
    pub p_requires_input: Option<i64>,
    pub p_input_prompt: Option<String>,
    pub p_description: Option<String>,
    pub p_image_url: Option<String>,
    pub p_delivery_type: Option<String>,
    pub p_file_path: Option<String>,
    pub p_file_name: Option<String>,
    pub p_file_mime: Option<String>,
    pub p_category: Option<String>,
    pub p_button_emoji: Option<String>,
    pub p_button_custom_emoji_id: Option<String>,
    pub p_created_at: Option<String>,
}

#[allow(dead_code)]
pub fn map_join_row(row: OrderJoinRow) -> OrderWithProduct {
    OrderWithProduct {
        order: Order {
            id: row.id,
            user_id: row.user_id,
            chat_id: row.chat_id,
            product_id: row.product_id,
            qty: row.qty,
            amount: row.amount,
            status: row.status,
            bank_memo: row.bank_memo,
            created_at: row.created_at,
            paid_at: row.paid_at,
            payment_tx_id: row.payment_tx_id,
            delivered_data: row.delivered_data,
            reserved_item_ids: row.reserved_item_ids,
            customer_input: row.customer_input,
            plan_id: row.plan_id,
            plan_label: row.plan_label,
            plan_months: row.plan_months,
            plan_price: row.plan_price,
            reservation_mode: row.reservation_mode,
        },
        product: Product {
            id: row.p_id,
            name: row.p_name,
            price: row.p_price,
            is_active: row.p_is_active,
            requires_input: row.p_requires_input,
            input_prompt: row.p_input_prompt,
            description: row.p_description,
            image_url: row.p_image_url,
            delivery_type: row.p_delivery_type,
            file_path: row.p_file_path,
            file_name: row.p_file_name,
            file_mime: row.p_file_mime,
            category_id: None,
            category: row.p_category,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: row.p_button_emoji,
            button_custom_emoji_id: row.p_button_custom_emoji_id,
            created_at: row.p_created_at,
            sort_order: None,
            show_sold_count: Some(0),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderRiskSummary {
    pub total_orders: i64,
    pub unpaid_orders: i64,
}

pub fn apply_order_filters(
    builder: &mut QueryBuilder<'_, Sqlite>,
    status: Option<OrderStatus>,
    query: Option<String>,
    from: Option<String>,
    to: Option<String>,
) {
    builder.push(" WHERE 1=1 ");

    if let Some(s) = status {
        builder.push(" AND o.status = ");
        builder.push_bind(s.to_string());
    }
    if let Some(q) = query {
        let like = format!("%{}%", q);
        builder.push(" AND (o.id LIKE ");
        builder.push_bind(like.clone());
        builder.push(" OR o.bank_memo LIKE ");
        builder.push_bind(like);
        builder.push(")");
    }
    if let Some(f) = from {
        builder.push(" AND o.created_at >= ");
        builder.push_bind(f);
    }
    if let Some(t) = to {
        builder.push(" AND o.created_at <= ");
        builder.push_bind(t);
    }
}

pub async fn cancel_order_for_user(
    pool: &SqlitePool,
    order_id: &str,
    user_id: Option<i64>,
) -> Result<bool> {
    let Some(uid) = user_id else {
        return Ok(false);
    };
    let result = sqlx::query(
        r#"UPDATE orders
        SET status = 'cancel'
        WHERE id = ? AND user_id = ? AND status = 'pending'"#,
    )
    .bind(order_id)
    .bind(uid)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn insert_order(pool: &SqlitePool, order: &Order) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO orders
        (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&order.id)
    .bind(order.user_id)
    .bind(order.chat_id)
    .bind(order.product_id)
    .bind(order.qty)
    .bind(order.amount)
    .bind(order.status.to_string())
    .bind(&order.bank_memo)
    .bind(&order.created_at)
    .bind(&order.paid_at)
    .bind(&order.payment_tx_id)
    .bind(&order.delivered_data)
    .bind(&order.reserved_item_ids)
    .bind(&order.customer_input)
    .bind(order.plan_id)
    .bind(&order.plan_label)
    .bind(order.plan_months)
    .bind(order.plan_price)
    .bind(order.reservation_mode.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_order_tx(tx: &mut Transaction<'_, Sqlite>, order: &Order) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO orders
        (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&order.id)
    .bind(order.user_id)
    .bind(order.chat_id)
    .bind(order.product_id)
    .bind(order.qty)
    .bind(order.amount)
    .bind(order.status.to_string())
    .bind(&order.bank_memo)
    .bind(&order.created_at)
    .bind(&order.paid_at)
    .bind(&order.payment_tx_id)
    .bind(&order.delivered_data)
    .bind(&order.reserved_item_ids)
    .bind(&order.customer_input)
    .bind(order.plan_id)
    .bind(&order.plan_label)
    .bind(order.plan_months)
    .bind(order.plan_price)
    .bind(order.reservation_mode.to_string())
    .execute(tx.as_mut())
    .await?;

    Ok(())
}

pub async fn get_order(pool: &SqlitePool, order_id: &str) -> Result<Option<Order>> {
    let order = sqlx::query_as::<sqlx::Sqlite, Order>(
        r#"SELECT id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode
        FROM orders
        WHERE id = ?"#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?;

    Ok(order)
}

pub async fn get_order_with_product(
    pool: &SqlitePool,
    order_id: &str,
) -> Result<Option<OrderWithProduct>> {
    let row = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.id = ?"#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_join_row))
}

pub async fn list_orders_admin(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    status: Option<OrderStatus>,
    query: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<OrderWithProduct>> {
    let mut builder = QueryBuilder::new(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id"#,
    );
    apply_order_filters(
        &mut builder,
        status,
        query.map(|s| s.to_string()),
        from.map(|s| s.to_string()),
        to.map(|s| s.to_string()),
    );
    builder
        .push(" ORDER BY o.created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = builder
        .build_query_as::<OrderJoinRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn count_orders_admin(
    pool: &SqlitePool,
    status: Option<OrderStatus>,
    query: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<i64> {
    let mut builder = QueryBuilder::new(
        r#"SELECT COUNT(1) FROM orders o JOIN products p ON p.id = o.product_id"#,
    );
    apply_order_filters(
        &mut builder,
        status,
        query.map(|s| s.to_string()),
        from.map(|s| s.to_string()),
        to.map(|s| s.to_string()),
    );
    let count = builder.build_query_scalar::<i64>().fetch_one(pool).await?;
    Ok(count)
}

pub async fn order_risk_summary(
    pool: &SqlitePool,
    user_id: i64,
    since: &str,
) -> Result<OrderRiskSummary> {
    let total_orders = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM orders WHERE user_id = ? AND created_at >= ?",
    )
    .bind(user_id)
    .bind(since)
    .fetch_one(pool)
    .await?;

    let unpaid_orders = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM orders WHERE user_id = ? AND created_at >= ? AND status IN ('expired', 'cancel')",
    )
    .bind(user_id)
    .bind(since)
    .fetch_one(pool)
    .await?;

    Ok(OrderRiskSummary {
        total_orders,
        unpaid_orders,
    })
}

pub async fn insert_order_risk_event(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    chat_id: i64,
    event_type: &str,
    reason: &str,
    window_started_at: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO order_risk_events
        (user_id, chat_id, event_type, reason, window_started_at)
        VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(user_id)
    .bind(chat_id)
    .bind(event_type)
    .bind(reason)
    .bind(window_started_at)
    .execute(tx.as_mut())
    .await?;

    Ok(())
}

pub async fn list_orders_for_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<OrderWithProduct>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.user_id = ?
        ORDER BY o.created_at DESC
        LIMIT ?"#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn list_paid_orders_for_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<OrderWithProduct>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.user_id = ? AND o.status = 'paid'
        ORDER BY o.paid_at DESC, o.created_at DESC
        LIMIT ?"#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn get_paid_order_for_user(
    pool: &SqlitePool,
    order_id: &str,
    user_id: i64,
) -> Result<Option<OrderWithProduct>> {
    let row = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.id = ? AND o.user_id = ? AND o.status = 'paid'"#,
    )
    .bind(order_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_join_row))
}

#[allow(clippy::too_many_arguments)]
pub async fn create_order_support_request(
    pool: &SqlitePool,
    order_id: &str,
    user_id: i64,
    chat_id: i64,
    username: Option<&str>,
    product_name: &str,
    bank_memo: &str,
    amount: i64,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO order_support_requests
            (order_id, user_id, chat_id, username, product_name, bank_memo, amount, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(order_id)
    .bind(user_id)
    .bind(chat_id)
    .bind(username)
    .bind(product_name)
    .bind(bank_memo)
    .bind(amount)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_recent_order_support_requests(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<OrderSupportRequest>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, OrderSupportRequest>(
        r#"SELECT
            id,
            order_id,
            user_id,
            chat_id,
            username,
            product_name,
            bank_memo,
            amount,
            created_at
        FROM order_support_requests
        ORDER BY created_at DESC, id DESC
        LIMIT ?"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn find_order_by_memo(pool: &SqlitePool, memo: &str) -> Result<Option<OrderWithProduct>> {
    let row = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category as p_category,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.bank_memo = ?"#,
    )
    .bind(memo)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_join_row))
}

pub async fn mark_order_paid(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    tx_id: &str,
    paid_at: DateTime<Utc>,
    delivered_data: Option<&str>,
    reserved_item_ids: Option<&str>,
) -> Result<()> {
    let _result: sqlx::sqlite::SqliteQueryResult = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
            SET status = 'paid', payment_tx_id = ?, paid_at = ?, delivered_data = ?, reserved_item_ids = ?
            WHERE id = ? AND status = 'pending'"#,
            )
            .bind(tx_id)
            .bind(paid_at.to_rfc3339())
            .bind(delivered_data)
            .bind(reserved_item_ids)
            .bind(order_id),
        )
        .await?;

    if _result.rows_affected() == 0 {
        return Err(anyhow!("order not found or not pending"));
    }

    Ok(())
}

pub async fn update_order_status(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    status: OrderStatus,
) -> Result<()> {
    let _result: sqlx::sqlite::SqliteQueryResult = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
                SET status = ?
                WHERE id = ?"#,
            )
            .bind(status.to_string())
            .bind(order_id),
        )
        .await?;

    if _result.rows_affected() == 0 {
        return Err(anyhow!("order not found"));
    }

    Ok(())
}

pub async fn update_order_status_with_data(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    status: OrderStatus,
    delivered_data: Option<&str>,
    reserved_item_ids: Option<&str>,
) -> Result<()> {
    let _result: sqlx::sqlite::SqliteQueryResult = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
                SET status = ?, delivered_data = ?, reserved_item_ids = ?
                WHERE id = ?"#,
            )
            .bind(status.to_string())
            .bind(delivered_data)
            .bind(reserved_item_ids)
            .bind(order_id),
        )
        .await?;

    if _result.rows_affected() == 0 {
        return Err(anyhow!("order not found"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed_order(pool: &SqlitePool, status: OrderStatus) -> Order {
        sqlx::query(
            "INSERT OR IGNORE INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)",
        )
        .bind(1_i64)
        .bind("Test")
        .bind(10_000_i64)
        .bind(1_i64)
        .execute(pool)
        .await
        .unwrap();

        let mut order = Order::new(
            100,
            200,
            1,
            1,
            10_000,
            format!("DH{}", uuid::Uuid::new_v4().simple()),
            None,
            None,
            None,
            None,
            None,
        );
        order.status = status;
        insert_order(pool, &order).await.unwrap();
        order
    }

    #[tokio::test]
    async fn mark_order_paid_rejects_non_pending_order() {
        let pool = test_pool().await;
        let order = seed_order(&pool, OrderStatus::Paid).await;

        let mut tx = pool.begin().await.unwrap();
        let result =
            mark_order_paid(&mut tx, &order.id, "wallet", Utc::now(), Some("data"), None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_paid_orders_for_user_returns_only_successful_orders() {
        let pool = test_pool().await;
        let paid = seed_order(&pool, OrderStatus::Paid).await;
        let _pending = seed_order(&pool, OrderStatus::Pending).await;

        let orders = list_paid_orders_for_user(&pool, paid.user_id, 10)
            .await
            .unwrap();

        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].order.id, paid.id);
        assert_eq!(orders[0].order.status, OrderStatus::Paid);
    }

    #[tokio::test]
    async fn get_paid_order_for_user_rejects_other_users_and_unpaid_orders() {
        let pool = test_pool().await;
        let paid = seed_order(&pool, OrderStatus::Paid).await;
        let pending = seed_order(&pool, OrderStatus::Pending).await;

        assert!(
            get_paid_order_for_user(&pool, &paid.id, paid.user_id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            get_paid_order_for_user(&pool, &paid.id, paid.user_id + 1)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            get_paid_order_for_user(&pool, &pending.id, pending.user_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn insert_and_get_order_preserves_no_reserve_mode() {
        let pool = test_pool().await;
        let mut order = seed_order(&pool, OrderStatus::Pending).await;
        order.id = format!("DH{}", uuid::Uuid::new_v4().simple());
        order.bank_memo = format!("MEMO{}", uuid::Uuid::new_v4().simple());
        order.reservation_mode = crate::domains::orders::models::OrderReservationMode::NoReserve;

        insert_order(&pool, &order).await.unwrap();

        let saved = get_order(&pool, &order.id).await.unwrap().unwrap();
        assert_eq!(
            saved.reservation_mode,
            crate::domains::orders::models::OrderReservationMode::NoReserve
        );
    }

    #[tokio::test]
    async fn risk_summary_counts_recent_unpaid_orders() {
        let pool = test_pool().await;
        let user_id = 700;
        let since = "2026-05-24T00:00:00+00:00";

        for status in [
            OrderStatus::Expired,
            OrderStatus::Cancel,
            OrderStatus::Cancel,
            OrderStatus::Paid,
        ] {
            let mut order = seed_order(&pool, status).await;
            order.id = format!("DH{}", uuid::Uuid::new_v4().simple());
            order.user_id = user_id;
            order.bank_memo = format!("MEMO{}", uuid::Uuid::new_v4().simple());
            order.created_at = "2026-05-24T01:00:00+00:00".to_string();
            insert_order(&pool, &order).await.unwrap();
        }

        let mut old = seed_order(&pool, OrderStatus::Expired).await;
        old.id = format!("DH{}", uuid::Uuid::new_v4().simple());
        old.user_id = user_id;
        old.bank_memo = format!("MEMO{}", uuid::Uuid::new_v4().simple());
        old.created_at = "2026-05-23T01:00:00+00:00".to_string();
        insert_order(&pool, &old).await.unwrap();

        let summary = order_risk_summary(&pool, user_id, since).await.unwrap();

        assert_eq!(summary.total_orders, 4);
        assert_eq!(summary.unpaid_orders, 3);
    }
}
