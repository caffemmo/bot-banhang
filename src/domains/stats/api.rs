use std::collections::HashMap;

use axum::extract::{Query, State};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::Serialize;

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::stats::repo;

#[derive(Serialize, Clone)]
pub struct RevenueResponse {
    pub from: String,
    pub to: String,
    pub amount: i64,
    pub reset_at: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct MonthlyRevenueItem {
    pub month: String,
    pub amount: i64,
}

#[derive(Serialize, Clone)]
pub struct MonthlyRevenueResponse {
    pub items: Vec<MonthlyRevenueItem>,
}

pub async fn revenue_last_days(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<RevenueResponse> {
    let to = Utc::now();
    let (from, reset_at) = if params.get("scope").map(String::as_str) == Some("reset") {
        reset_revenue_window(&ctx.pool, to).await?
    } else {
        let days: i64 = params
            .get("days")
            .and_then(|s| s.parse().ok())
            .filter(|d| *d > 0 && *d <= 365)
            .unwrap_or(7);
        (to - Duration::days(days), None)
    };
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let amount = repo::sum_paid_between(&ctx.pool, &from_str, &to_str)
        .await
        .map_err(|e| ApiError::internal(format!("sum revenue failed: {e}")))?;

    Ok(ok(RevenueResponse {
        from: from_str,
        to: to_str,
        amount,
        reset_at,
    }))
}

pub async fn reset_revenue_period(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
) -> ApiResult<RevenueResponse> {
    let now = Utc::now();
    let reset_at = now.to_rfc3339();
    repo::set_revenue_reset_at(&ctx.pool, &reset_at)
        .await
        .map_err(|e| ApiError::internal(format!("reset revenue period failed: {e}")))?;

    Ok(ok(RevenueResponse {
        from: reset_at.clone(),
        to: reset_at.clone(),
        amount: 0,
        reset_at: Some(reset_at),
    }))
}

pub async fn monthly_revenue(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<MonthlyRevenueResponse> {
    let limit: i64 = params
        .get("months")
        .and_then(|s| s.parse().ok())
        .filter(|value| *value > 0 && *value <= 60)
        .unwrap_or(12);
    let items = repo::list_monthly_revenue(&ctx.pool, limit)
        .await
        .map_err(|e| ApiError::internal(format!("list monthly revenue failed: {e}")))?
        .into_iter()
        .map(|item| MonthlyRevenueItem {
            month: item.month,
            amount: item.amount,
        })
        .collect();

    Ok(ok(MonthlyRevenueResponse { items }))
}

async fn reset_revenue_window(
    pool: &sqlx::SqlitePool,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, Option<String>), ApiError> {
    let reset_at = repo::get_revenue_reset_at(pool)
        .await
        .map_err(|e| ApiError::internal(format!("load revenue reset failed: {e}")))?;
    if let Some(value) = reset_at {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(&value) {
            return Ok((parsed.with_timezone(&Utc), Some(value)));
        }
    }

    Ok((current_month_start(now), None))
}

fn current_month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use chrono::{Datelike, TimeZone};
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use teloxide::Bot;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_config() -> crate::config::Config {
        crate::config::Config {
            telegram_token: "TEST".into(),
            database_url: "sqlite::memory:".into(),
            bank_name: "VCB".into(),
            bank_account: Some("0000".into()),
            bank_account_name: None,
            webhook_secret: "webhook".into(),
            admin_jwt_secret: "test-secret-that-is-long-enough-for-hmac".into(),
            admin_setup_code: "SETUP-123".into(),
            admin_cookie_secure: false,
            base_url: None,
            i18n_dir: "i18n".to_string(),
            port: 0,
            crypto: crate::config::CryptoConfig::default(),
        }
    }

    #[tokio::test]
    async fn revenue_handler_sums_paid() {
        let pool = test_pool().await;
        // seed product
        sqlx::query("INSERT INTO products (name, price, is_active) VALUES ('Test', 1000, 1)")
            .execute(&pool)
            .await
            .unwrap();
        let product_id: i64 = sqlx::query_scalar("SELECT id FROM products LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        // paid order inside window
        sqlx::query(
            r#"INSERT INTO orders
            (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price)
            VALUES (?, ?, ?, ?, ?, ?, 'paid', 'MEMO1', ?, ?, 'tx1', NULL, NULL, NULL, NULL, NULL, NULL, NULL)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(1_i64)
        .bind(1_i64)
        .bind(product_id)
        .bind(1_i64)
        .bind(5000_i64)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        // pending order should not count
        sqlx::query(
            r#"INSERT INTO orders
            (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', 'MEMO2', ?, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(1_i64)
        .bind(1_i64)
        .bind(product_id)
        .bind(1_i64)
        .bind(7000_i64)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        let ctx = Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool: pool.clone(),
            config: test_config(),
            configs: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(std::sync::RwLock::new(
                crate::bot::texts::BotTexts::default(),
            )),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        });

        let mut headers = axum::http::HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());
        let axum::Json(resp) = revenue_last_days(State(ctx), headers, Query(HashMap::new()))
            .await
            .unwrap();
        assert_eq!(resp.data.amount, 5000);
    }

    async fn seed_product(pool: &SqlitePool) -> i64 {
        sqlx::query("INSERT INTO products (name, price, is_active) VALUES ('Test', 1000, 1)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT id FROM products LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn seed_paid_order(
        pool: &SqlitePool,
        product_id: i64,
        memo: &str,
        amount: i64,
        paid_at: &str,
    ) {
        sqlx::query(
            r#"INSERT INTO orders
            (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price)
            VALUES (?, ?, ?, ?, ?, ?, 'paid', ?, ?, ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(1_i64)
        .bind(1_i64)
        .bind(product_id)
        .bind(1_i64)
        .bind(amount)
        .bind(memo)
        .bind(paid_at)
        .bind(paid_at)
        .bind(format!("tx-{memo}"))
        .execute(pool)
        .await
        .unwrap();
    }

    fn test_context(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool,
            config: test_config(),
            configs: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(std::sync::RwLock::new(
                crate::bot::texts::BotTexts::default(),
            )),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    #[tokio::test]
    async fn revenue_reset_scope_defaults_to_current_month() {
        let pool = test_pool().await;
        let product_id = seed_product(&pool).await;
        let now = Utc::now();
        let month_start = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .unwrap();
        let before_month = (month_start - Duration::seconds(1)).to_rfc3339();
        let inside_month = now.to_rfc3339();
        seed_paid_order(&pool, product_id, "OLDMONTH", 9000, &before_month).await;
        seed_paid_order(&pool, product_id, "THISMONTH", 4000, &inside_month).await;

        let ctx = test_context(pool);
        let params = HashMap::from([("scope".to_string(), "reset".to_string())]);
        let axum::Json(resp) =
            revenue_last_days(State(ctx), axum::http::HeaderMap::new(), Query(params))
                .await
                .unwrap();

        assert_eq!(resp.data.amount, 4000);
        assert_eq!(resp.data.from, month_start.to_rfc3339());
        assert_eq!(resp.data.reset_at, None);
    }

    #[tokio::test]
    async fn revenue_reset_scope_uses_saved_reset_timestamp() {
        let pool = test_pool().await;
        let product_id = seed_product(&pool).await;
        let reset_at = (Utc::now() - Duration::hours(1)).to_rfc3339();
        let before_reset = (Utc::now() - Duration::hours(2)).to_rfc3339();
        let after_reset = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        sqlx::query("INSERT INTO app_configs (key, value) VALUES ('stats_revenue_reset_at', ?)")
            .bind(&reset_at)
            .execute(&pool)
            .await
            .unwrap();
        seed_paid_order(&pool, product_id, "BEFORERESET", 8000, &before_reset).await;
        seed_paid_order(&pool, product_id, "AFTERRESET", 3000, &after_reset).await;

        let ctx = test_context(pool);
        let params = HashMap::from([("scope".to_string(), "reset".to_string())]);
        let axum::Json(resp) =
            revenue_last_days(State(ctx), axum::http::HeaderMap::new(), Query(params))
                .await
                .unwrap();

        assert_eq!(resp.data.amount, 3000);
        assert_eq!(resp.data.from, reset_at);
        assert_eq!(resp.data.reset_at, Some(reset_at));
    }

    #[tokio::test]
    async fn reset_revenue_period_saves_current_timestamp() {
        let pool = test_pool().await;
        let ctx = test_context(pool.clone());

        let axum::Json(resp) = reset_revenue_period(State(ctx), axum::http::HeaderMap::new())
            .await
            .unwrap();
        let saved: String = sqlx::query_scalar(
            "SELECT value FROM app_configs WHERE key = 'stats_revenue_reset_at'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(resp.data.reset_at, Some(saved.clone()));
        assert_eq!(resp.data.from, saved);
        assert_eq!(resp.data.amount, 0);
    }

    #[tokio::test]
    async fn monthly_revenue_lists_paid_revenue_grouped_by_month() {
        let pool = test_pool().await;
        let product_id = seed_product(&pool).await;
        seed_paid_order(
            &pool,
            product_id,
            "MAYONE",
            2000,
            "2026-05-02T03:00:00+00:00",
        )
        .await;
        seed_paid_order(
            &pool,
            product_id,
            "MAYTWO",
            3000,
            "2026-05-25T04:00:00+00:00",
        )
        .await;
        seed_paid_order(
            &pool,
            product_id,
            "JUNEONE",
            7000,
            "2026-06-01T01:00:00+00:00",
        )
        .await;
        sqlx::query(
            r#"INSERT INTO orders
            (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', 'PENDINGMONTH', '2026-06-01T02:00:00+00:00', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(1_i64)
        .bind(1_i64)
        .bind(product_id)
        .bind(1_i64)
        .bind(9000_i64)
        .execute(&pool)
        .await
        .unwrap();

        let ctx = test_context(pool);
        let params = HashMap::from([("months".to_string(), "12".to_string())]);
        let axum::Json(resp) =
            monthly_revenue(State(ctx), axum::http::HeaderMap::new(), Query(params))
                .await
                .unwrap();

        assert_eq!(resp.data.items.len(), 2);
        assert_eq!(resp.data.items[0].month, "2026-06");
        assert_eq!(resp.data.items[0].amount, 7000);
        assert_eq!(resp.data.items[1].month, "2026-05");
        assert_eq!(resp.data.items[1].amount, 5000);
    }

    #[tokio::test]
    async fn revenue_route_requires_token() {
        let pool = test_pool().await;
        let ctx = Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool,
            config: test_config(),
            configs: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(std::sync::RwLock::new(
                crate::bot::texts::BotTexts::default(),
            )),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        });

        let app = Router::new()
            .route("/api/stats/revenue", get(revenue_last_days))
            .layer(axum::middleware::from_fn_with_state(
                ctx.clone(),
                crate::domains::auth::api::require_admin_session,
            ))
            .with_state(ctx);

        let res = app
            .oneshot(
                axum::http::Request::get("/api/stats/revenue")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}

use axum::Router;
use axum::routing::{get, post};
use std::sync::Arc;

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/api/admin/stats/revenue", get(revenue_last_days))
        .route("/api/admin/stats/revenue/reset", post(reset_revenue_period))
        .route("/api/admin/stats/revenue/monthly", get(monthly_revenue))
}
