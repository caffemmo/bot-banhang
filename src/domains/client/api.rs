use std::sync::Arc;

use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use chrono::Utc;
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::orders::admin_notify::notify_admins_order_paid;
use crate::domains::orders::api as orders_api;
use crate::domains::orders::fulfillment::PaymentSource;
use crate::domains::orders::models::{Order, OrderStatus, OrderWithProduct};
use crate::domains::orders::repo as orders_repo;
use crate::domains::products::models::{Product, ProductPlan};
use crate::domains::products::repo as products_repo;
use crate::domains::wallet::repo as wallet_repo;

use super::repo as client_repo;

#[derive(Debug, Clone, Copy)]
struct ClientAuth {
    chat_id: i64,
}

#[derive(Debug, Serialize)]
pub struct ClientWalletResponse {
    pub chat_id: i64,
    pub balance: i64,
    pub balance_display: String,
}

#[derive(Debug, Serialize)]
pub struct ClientProductItem {
    pub id: i64,
    pub name: String,
    pub price: i64,
    pub category: Option<String>,
    pub description: Option<String>,
    pub delivery_type: String,
    pub stock_count: i64,
    pub plans: Vec<ProductPlan>,
}

#[derive(Debug, Deserialize)]
pub struct ClientBuyPayload {
    pub product_id: i64,
    pub qty: Option<i64>,
    pub plan_id: Option<i64>,
    pub customer_input: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientOrderResponse {
    pub order_id: String,
    pub product_id: i64,
    pub qty: i64,
    pub amount: i64,
    pub amount_display: String,
    pub balance_after: i64,
    pub delivered_data: Option<String>,
}

fn bearer_api_key(headers: &HeaderMap) -> Result<(i64, String), ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or_else(ApiError::unauthorized)?;
    let (chat_id, secret) = token.split_once(':').ok_or_else(ApiError::unauthorized)?;
    let chat_id = chat_id
        .trim()
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(ApiError::unauthorized());
    }
    Ok((chat_id, secret.to_string()))
}

async fn require_client_auth(
    ctx: &AppContext,
    headers: &HeaderMap,
) -> Result<ClientAuth, ApiError> {
    let (chat_id, token) = bearer_api_key(headers)?;
    let valid = client_repo::verify_api_key(&ctx.pool, chat_id, &token)
        .await
        .map_err(|e| ApiError::internal(format!("verify api key failed: {e}")))?;
    if !valid {
        return Err(ApiError::unauthorized());
    }
    Ok(ClientAuth { chat_id })
}

pub async fn list_products(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<Vec<ClientProductItem>> {
    let _auth = require_client_auth(&ctx, &headers).await?;
    let products = products_repo::list_products(&ctx.pool, 500, 0)
        .await
        .map_err(|e| ApiError::internal(format!("list products failed: {e}")))?;

    let mut items = Vec::new();
    for product in products {
        let delivery_type = orders_api::product_delivery_type(&product).to_string();
        let stock_count = products_repo::count_product_items(&ctx.pool, product.id)
            .await
            .unwrap_or(0);
        let plans = products_repo::list_product_plans(&ctx.pool, product.id)
            .await
            .unwrap_or_default();
        items.push(ClientProductItem {
            id: product.id,
            name: product.name,
            price: product.price,
            category: product.category,
            description: product.description,
            delivery_type,
            stock_count,
            plans,
        });
    }
    Ok(ok(items))
}

pub async fn wallet(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<ClientWalletResponse> {
    let auth = require_client_auth(&ctx, &headers).await?;
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, auth.chat_id)
        .await
        .map_err(|e| ApiError::internal(format!("get wallet failed: {e}")))?;
    Ok(ok(ClientWalletResponse {
        chat_id: auth.chat_id,
        balance: wallet.balance,
        balance_display: format_vnd(wallet.balance),
    }))
}

pub async fn buy(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Json(payload): Json<ClientBuyPayload>,
) -> ApiResult<ClientOrderResponse> {
    let auth = require_client_auth(&ctx, &headers).await?;
    let response = buy_with_wallet(&ctx, auth.chat_id, payload)
        .await
        .map_err(client_buy_error)?;
    Ok(ok(response))
}

fn client_buy_error(err: anyhow::Error) -> ApiError {
    let msg = err.to_string();
    if msg.contains("not found") {
        ApiError::not_found(msg)
    } else if msg.contains("không đủ")
        || msg.contains("not enough")
        || msg.contains("qty")
        || msg.contains("plan")
        || msg.contains("stock")
    {
        ApiError::validation(msg)
    } else {
        ApiError::internal(msg)
    }
}

async fn buy_with_wallet(
    ctx: &AppContext,
    chat_id: i64,
    payload: ClientBuyPayload,
) -> Result<ClientOrderResponse> {
    let qty = payload.qty.unwrap_or(1);
    if qty <= 0 {
        return Err(anyhow!("qty must be > 0"));
    }

    let product = products_repo::get_product(&ctx.pool, payload.product_id)
        .await?
        .filter(|p| p.is_active.unwrap_or(1) == 1)
        .ok_or_else(|| anyhow!("product not found"))?;
    let delivery_type = orders_api::product_delivery_type(&product).to_string();
    let plans = products_repo::list_product_plans(&ctx.pool, product.id).await?;
    let selected_plan = select_plan(payload.plan_id, &plans, &delivery_type)?;
    let order_qty = order_qty_for_delivery_type(&delivery_type, qty, selected_plan);
    let amount = selected_plan
        .as_ref()
        .map(|plan| plan.price)
        .unwrap_or(product.price * qty);

    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, chat_id).await?;
    if wallet.balance < amount {
        return Err(anyhow!(
            "wallet balance is not enough: current {}, required {}",
            wallet.balance,
            amount
        ));
    }

    let mut order = Order::new(
        chat_id,
        chat_id,
        product.id,
        order_qty,
        amount,
        new_api_memo(),
        payload.customer_input.clone(),
        selected_plan.as_ref().map(|plan| plan.id),
        selected_plan.as_ref().map(|plan| plan.label.clone()),
        selected_plan.as_ref().map(|plan| plan.months),
        selected_plan.as_ref().map(|plan| plan.price),
    );

    let mut tx = ctx.pool.begin().await?;
    let (delivered_data, reserved_item_ids) =
        reserve_delivery_data(&mut tx, &product, &delivery_type, order_qty, &payload).await?;
    order.delivered_data = Some(delivered_data.clone());
    order.reserved_item_ids = reserved_item_ids;
    products_repo::insert_order_tx(&mut tx, &order).await?;
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        chat_id,
        amount,
        &order.id,
        Some("client_api_wallet_purchase"),
    )
    .await?;
    let paid_at = Utc::now();
    orders_repo::mark_order_paid(
        &mut tx,
        &order.id,
        "client_api_wallet",
        paid_at,
        Some(&delivered_data),
        order.reserved_item_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    order.status = OrderStatus::Paid;
    order.payment_tx_id = Some("client_api_wallet".to_string());
    order.paid_at = Some(paid_at.to_rfc3339());
    let paid_order = OrderWithProduct {
        order: order.clone(),
        product: product.clone(),
    };
    if let Err(err) = notify_admins_order_paid(
        ctx,
        &paid_order,
        "client_api_wallet",
        paid_at,
        &PaymentSource::ClientApiWallet,
    )
    .await
    {
        tracing::error!(
            "send paid-order admin notification after client API wallet payment failed: {err}"
        );
    }

    Ok(ClientOrderResponse {
        order_id: order.id,
        product_id: product.id,
        qty: order_qty,
        amount,
        amount_display: format_vnd(amount),
        balance_after,
        delivered_data: Some(delivered_data),
    })
}

fn select_plan<'a>(
    plan_id: Option<i64>,
    plans: &'a [ProductPlan],
    delivery_type: &str,
) -> Result<Option<&'a ProductPlan>> {
    if delivery_type != "manual_input" {
        return Ok(None);
    }
    if plans.is_empty() {
        return Ok(None);
    }
    let plan_id = plan_id.ok_or_else(|| anyhow!("plan_id is required for this product"))?;
    plans
        .iter()
        .find(|plan| plan.id == plan_id)
        .map(Some)
        .ok_or_else(|| anyhow!("plan not found"))
}

fn order_qty_for_delivery_type(
    delivery_type: &str,
    requested_qty: i64,
    plan: Option<&ProductPlan>,
) -> i64 {
    if delivery_type == "manual_input" {
        plan.map(|p| p.months).unwrap_or(requested_qty)
    } else {
        requested_qty
    }
}

async fn reserve_delivery_data(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product: &Product,
    delivery_type: &str,
    qty: i64,
    payload: &ClientBuyPayload,
) -> Result<(String, Option<String>)> {
    if delivery_type == "manual_input" {
        let info = payload
            .customer_input
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "N/A".to_string());
        return Ok((format!("info: {info}"), None));
    }

    let reserved = products_repo::take_product_items(tx, product.id, qty).await?;
    let data = reserved
        .iter()
        .map(|i| i.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim().is_empty() {
        return Err(anyhow!("stock is empty"));
    }
    let reserved_ids = reserved
        .iter()
        .map(|item| item.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    Ok((data, Some(reserved_ids)))
}

fn new_api_memo() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect::<String>()
        .to_ascii_uppercase();
    format!("API{suffix}")
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/client/products", get(list_products))
        .route("/api/client/wallet", get(wallet))
        .route("/api/client/orders", post(buy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, Response, StatusCode, header},
    };
    use serde_json::{Value, json};
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::sync::{Arc, RwLock};
    use teloxide::Bot;
    use tower::ServiceExt;

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

    fn test_context(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool,
            config: test_config(),
            configs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(RwLock::new(crate::bot::texts::BotTexts::default())),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    async fn response_json(response: Response<Body>) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn auth_header(chat_id: i64, token: &str) -> String {
        format!("Bearer {chat_id}:{token}")
    }

    async fn set_wallet_balance(pool: &SqlitePool, user_id: i64, balance: i64) {
        sqlx::query(
            "INSERT OR REPLACE INTO wallets (user_id, balance, updated_at)
            VALUES (?, ?, datetime('now'))",
        )
        .bind(user_id)
        .bind(balance)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn wallet_requires_current_api_key() {
        let pool = test_pool().await;
        let old_token = client_repo::create_or_replace_api_key(&pool, 42)
            .await
            .unwrap();
        let new_token = client_repo::create_or_replace_api_key(&pool, 42)
            .await
            .unwrap();
        set_wallet_balance(&pool, 42, 12_000).await;

        let app = router().with_state(test_context(pool));

        let missing = app
            .clone()
            .oneshot(
                Request::get("/api/client/wallet")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let old = app
            .clone()
            .oneshot(
                Request::get("/api/client/wallet")
                    .header(header::AUTHORIZATION, auth_header(42, &old_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old.status(), StatusCode::UNAUTHORIZED);

        let current = app
            .oneshot(
                Request::get("/api/client/wallet")
                    .header(header::AUTHORIZATION, auth_header(42, &new_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(current.status(), StatusCode::OK);
        let body = response_json(current).await;
        assert_eq!(body["data"]["balance"], 12_000);
    }

    #[tokio::test]
    async fn list_products_returns_stock_and_plans() {
        let pool = test_pool().await;
        let token = client_repo::create_or_replace_api_key(&pool, 42)
            .await
            .unwrap();
        let stock_product = products_repo::insert_product(
            &pool,
            "API Key",
            10_000,
            Some(1),
            Some(0),
            None,
            Some("Instant stock"),
            None,
            Some("stock_item"),
            None,
            None,
            None,
            None,
            Some("Keys"),
            None,
            None,
        )
        .await
        .unwrap();
        products_repo::insert_product_items(
            &pool,
            stock_product.id,
            &["secret-1".to_string(), "secret-2".to_string()],
        )
        .await
        .unwrap();
        let manual_product = products_repo::insert_product(
            &pool,
            "Service",
            0,
            Some(1),
            Some(1),
            Some("Email"),
            None,
            None,
            Some("manual_input"),
            None,
            None,
            None,
            None,
            Some("Services"),
            None,
            None,
        )
        .await
        .unwrap();
        products_repo::insert_product_plan(
            &pool,
            manual_product.id,
            "3 months",
            3,
            25_000,
            Some(1),
        )
        .await
        .unwrap();

        let response = router()
            .with_state(test_context(pool))
            .oneshot(
                Request::get("/api/client/products")
                    .header(header::AUTHORIZATION, auth_header(42, &token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        let products = body["data"].as_array().unwrap();
        let stock = products
            .iter()
            .find(|item| item["id"] == stock_product.id)
            .unwrap();
        assert_eq!(stock["stock_count"], 2);
        assert_eq!(stock["delivery_type"], "stock_item");

        let manual = products
            .iter()
            .find(|item| item["id"] == manual_product.id)
            .unwrap();
        assert_eq!(manual["delivery_type"], "manual_input");
        assert_eq!(manual["plans"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn buy_stock_product_uses_wallet_and_reserves_item() {
        let pool = test_pool().await;
        let token = client_repo::create_or_replace_api_key(&pool, 42)
            .await
            .unwrap();
        let product = products_repo::insert_product(
            &pool,
            "API Key",
            10_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            Some("stock_item"),
            None,
            None,
            None,
            None,
            Some("Keys"),
            None,
            None,
        )
        .await
        .unwrap();
        products_repo::insert_product_items(&pool, product.id, &["secret-1".to_string()])
            .await
            .unwrap();
        set_wallet_balance(&pool, 42, 20_000).await;

        let response = router()
            .with_state(test_context(pool.clone()))
            .oneshot(
                Request::post("/api/client/orders")
                    .header(header::AUTHORIZATION, auth_header(42, &token))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "product_id": product.id,
                            "qty": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        assert_eq!(body["data"]["amount"], 10_000);
        assert_eq!(body["data"]["balance_after"], 10_000);
        assert_eq!(body["data"]["delivered_data"], "secret-1");
        let order_id = body["data"]["order_id"].as_str().unwrap();

        let wallet = wallet_repo::get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 10_000);
        assert_eq!(
            products_repo::count_product_items(&pool, product.id)
                .await
                .unwrap(),
            0
        );
        let order = orders_repo::get_order(&pool, order_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(order.status.to_string(), "paid");
        assert_eq!(order.amount, 10_000);
    }

    #[tokio::test]
    async fn buy_manual_plan_uses_plan_price_once() {
        let pool = test_pool().await;
        let token = client_repo::create_or_replace_api_key(&pool, 42)
            .await
            .unwrap();
        let product = products_repo::insert_product(
            &pool,
            "VIP Service",
            0,
            Some(1),
            Some(1),
            Some("Email"),
            None,
            None,
            Some("manual_input"),
            None,
            None,
            None,
            None,
            Some("Services"),
            None,
            None,
        )
        .await
        .unwrap();
        let plan =
            products_repo::insert_product_plan(&pool, product.id, "3 months", 3, 25_000, Some(1))
                .await
                .unwrap();
        set_wallet_balance(&pool, 42, 30_000).await;

        let response = router()
            .with_state(test_context(pool.clone()))
            .oneshot(
                Request::post("/api/client/orders")
                    .header(header::AUTHORIZATION, auth_header(42, &token))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "product_id": product.id,
                            "plan_id": plan.id,
                            "qty": 99,
                            "customer_input": "user@example.test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        assert_eq!(body["data"]["qty"], 3);
        assert_eq!(body["data"]["amount"], 25_000);
        assert_eq!(body["data"]["balance_after"], 5_000);
        assert_eq!(body["data"]["delivered_data"], "info: user@example.test");
    }
}
