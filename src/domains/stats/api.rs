use std::collections::HashMap;

use axum::extract::{Query, State};
use chrono::{Duration, Utc};
use serde::Serialize;

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::products::repo;

#[derive(Serialize, Clone)]
pub struct RevenueResponse {
    pub from: String,
    pub to: String,
    pub amount: i64,
}

pub async fn revenue_last_days(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<RevenueResponse> {
    let days: i64 = params
        .get("days")
        .and_then(|s| s.parse().ok())
        .filter(|d| *d > 0 && *d <= 365)
        .unwrap_or(7);
    let to = Utc::now();
    let from = to - Duration::days(days);
    let amount = repo::sum_paid_between(
        &ctx.pool,
        from.to_rfc3339().as_str(),
        to.to_rfc3339().as_str(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("sum revenue failed: {e}")))?;

    Ok(ok(RevenueResponse {
        from: from.to_rfc3339(),
        to: to.to_rfc3339(),
        amount,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
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
use axum::routing::get;
use std::sync::Arc;

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new().route("/api/admin/stats/revenue", get(revenue_last_days))
}
