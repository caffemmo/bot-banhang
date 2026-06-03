use crate::domains::stats::models::{DashboardAggregatedStats, DashboardStats};
use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub const REVENUE_RESET_CONFIG_KEY: &str = "stats_revenue_reset_at";

#[derive(Debug, Clone)]
pub struct MonthlyRevenueRow {
    pub month: String,
    pub amount: i64,
}

pub async fn sum_paid_between(pool: &SqlitePool, from: &str, to: &str) -> Result<i64> {
    let total = sqlx::query_scalar::<_, i64>(
        r#"SELECT IFNULL(SUM(amount), 0)
           FROM orders
           WHERE status = 'paid'
             AND COALESCE(paid_at, created_at) >= ?
             AND COALESCE(paid_at, created_at) <= ?"#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    Ok(total)
}

pub async fn list_monthly_revenue(pool: &SqlitePool, limit: i64) -> Result<Vec<MonthlyRevenueRow>> {
    let rows = sqlx::query(
        r#"
        SELECT strftime('%Y-%m', COALESCE(paid_at, created_at)) AS month,
               COALESCE(SUM(amount), 0) AS amount
        FROM orders
        WHERE status = 'paid'
        GROUP BY month
        HAVING amount > 0
        ORDER BY month DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| MonthlyRevenueRow {
            month: row.try_get("month").unwrap_or_default(),
            amount: row.try_get("amount").unwrap_or(0),
        })
        .collect())
}

pub async fn get_revenue_reset_at(pool: &SqlitePool) -> Result<Option<String>> {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM app_configs WHERE key = ?")
        .bind(REVENUE_RESET_CONFIG_KEY)
        .fetch_optional(pool)
        .await?;

    Ok(value)
}

pub async fn set_revenue_reset_at(pool: &SqlitePool, reset_at: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO app_configs (key, value)
        VALUES (?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        "#,
    )
    .bind(REVENUE_RESET_CONFIG_KEY)
    .bind(reset_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_dashboard_stats(pool: &SqlitePool) -> Result<DashboardStats> {
    let row = sqlx::query(
        r#"
        SELECT 
            COALESCE(SUM(amount), 0) as total_revenue,
            COALESCE(SUM(CASE WHEN date(created_at) = date('now') THEN amount ELSE 0 END), 0) as today_revenue,
            COUNT(CASE WHEN status = 'pending' THEN 1 END) as pending_orders,
            COUNT(CASE WHEN status = 'paid' THEN 1 END) as completed_orders
        FROM orders
        "#
    )
    .fetch_one(pool)
    .await?;

    Ok(DashboardStats {
        total_revenue: row.try_get("total_revenue").unwrap_or(0),
        today_revenue: row.try_get("today_revenue").unwrap_or(0),
        pending_orders: row.try_get("pending_orders").unwrap_or(0),
        completed_orders: row.try_get("completed_orders").unwrap_or(0),
    })
}

pub async fn get_aggregated_stats(pool: &SqlitePool) -> Result<DashboardAggregatedStats> {
    let users_count: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT user_id) FROM orders")
        .fetch_one(pool)
        .await?;

    let products_count: i64 =
        sqlx::query_scalar("SELECT COUNT(id) FROM products WHERE is_active = 1")
            .fetch_one(pool)
            .await?;

    let plans_count: i64 = sqlx::query_scalar("SELECT COUNT(id) FROM product_plans")
        .fetch_one(pool)
        .await?;

    let pending_orders: i64 =
        sqlx::query_scalar("SELECT COUNT(id) FROM orders WHERE status = 'pending'")
            .fetch_one(pool)
            .await?;

    let lifetime_revenue: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0) FROM orders WHERE status = 'paid'")
            .fetch_one(pool)
            .await?;

    Ok(DashboardAggregatedStats {
        total_users: users_count,
        active_products: products_count,
        active_plans: plans_count,
        pending_orders,
        lifetime_revenue,
    })
}
