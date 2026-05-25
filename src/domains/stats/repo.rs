use crate::domains::stats::models::{DashboardAggregatedStats, DashboardStats};
use anyhow::Result;
use sqlx::{Row, SqlitePool};

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
