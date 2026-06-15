use std::sync::Arc;

use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
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
use crate::domains::products::usage_instructions;
use crate::domains::wallet::repo as wallet_repo;

use super::repo::{self, ChildBotRecord};

#[derive(Debug, Clone)]
struct ChildBotAuth {
    child_bot: ChildBotRecord,
}

#[derive(Debug, Serialize)]
pub struct ChildBotInfoResponse {
    pub child_bot_id: i64,
    pub owner_user_id: i64,
    pub bot_username: Option<String>,
    pub shop_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChildBotBalanceResponse {
    pub owner_user_id: i64,
    pub balance: i64,
    pub balance_display: String,
}

#[derive(Debug, Serialize)]
pub struct ChildBotOrderItem {
    pub order_id: String,
    pub buyer_user_id: i64,
    pub product_name: String,
    pub amount: i64,
    pub amount_display: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ChildBotProductItem {
    pub id: i64,
    pub name: String,
    pub price: i64,
    pub category: Option<String>,
    pub category_emoji: Option<String>,
    pub category_custom_emoji_id: Option<String>,
    pub button_emoji: Option<String>,
    pub button_custom_emoji_id: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub delivery_type: String,
    pub stock_count: i64,
    pub plans: Vec<ProductPlan>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePurchaseRequestPayload {
    pub telegram_id: i64,
    pub chat_id: Option<i64>,
    pub product_id: i64,
    pub qty: Option<i64>,
    pub plan_id: Option<i64>,
    pub customer_input: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PurchaseRequestResponse {
    pub request_id: i64,
    pub status: String,
    pub child_bot_id: i64,
    pub affiliate_user_id: i64,
    pub telegram_id: i64,
    pub product_id: i64,
    pub qty: i64,
    pub amount: i64,
    pub amount_display: String,
    pub confirmation_sent: bool,
    pub order_id: Option<String>,
    pub balance_after: Option<i64>,
    pub balance_after_display: Option<String>,
    pub delivered_data: Option<String>,
    pub usage_instructions: Option<String>,
}

struct DirectChildBotOrder {
    order_id: String,
    product_id: i64,
    qty: i64,
    amount: i64,
    balance_after: i64,
    delivered_data: String,
    usage_instructions: Option<String>,
}

fn bearer_token(headers: &HeaderMap) -> Result<String, ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or_else(ApiError::unauthorized)?
        .trim();
    if token.is_empty() {
        return Err(ApiError::unauthorized());
    }
    Ok(token.to_string())
}

async fn require_child_bot_auth(
    ctx: &AppContext,
    headers: &HeaderMap,
) -> Result<ChildBotAuth, ApiError> {
    let token = bearer_token(headers)?;
    let child_bot = repo::verify_api_key(&ctx.pool, &token)
        .await
        .map_err(|e| ApiError::internal(format!("verify child bot api key failed: {e}")))?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(ChildBotAuth { child_bot })
}

pub async fn me(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<ChildBotInfoResponse> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    Ok(ok(ChildBotInfoResponse {
        child_bot_id: auth.child_bot.id,
        owner_user_id: auth.child_bot.owner_user_id,
        bot_username: auth.child_bot.bot_username,
        shop_name: auth.child_bot.shop_name,
    }))
}

pub async fn balance(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<ChildBotBalanceResponse> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, auth.child_bot.owner_user_id)
        .await
        .map_err(|e| ApiError::internal(format!("get child bot owner wallet failed: {e}")))?;
    Ok(ok(ChildBotBalanceResponse {
        owner_user_id: auth.child_bot.owner_user_id,
        balance: wallet.balance,
        balance_display: format_vnd(wallet.balance),
    }))
}

pub async fn list_orders(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<Vec<ChildBotOrderItem>> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    let orders = repo::list_child_bot_order_summaries(&ctx.pool, auth.child_bot.id, 20)
        .await
        .map_err(|e| ApiError::internal(format!("list child bot orders failed: {e}")))?;
    Ok(ok(
        orders
            .into_iter()
            .map(|order| ChildBotOrderItem {
                order_id: order.order_id,
                buyer_user_id: order.buyer_user_id,
                product_name: order.product_name,
                amount: order.amount,
                amount_display: format_vnd(order.amount),
                created_at: order.created_at,
            })
            .collect(),
    ))
}

pub async fn list_products(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<Vec<ChildBotProductItem>> {
    let _auth = require_child_bot_auth(&ctx, &headers).await?;
    let products = products_repo::list_products(&ctx.pool, 500, 0)
        .await
        .map_err(|e| ApiError::internal(format!("list products failed: {e}")))?;

    let mut items = Vec::new();
    for product in products {
        if product.is_active.unwrap_or(1) != 1 {
            continue;
        }
        let delivery_type = orders_api::product_delivery_type(&product).to_string();
        let stock_count = products_repo::count_product_items(&ctx.pool, product.id)
            .await
            .unwrap_or(0);
        let plans = products_repo::list_product_plans(&ctx.pool, product.id)
            .await
            .unwrap_or_default();
        items.push(ChildBotProductItem {
            id: product.id,
            name: product.name,
            price: product.price,
            category: product.category,
            category_emoji: product.category_emoji,
            category_custom_emoji_id: product.category_custom_emoji_id,
            button_emoji: product.button_emoji,
            button_custom_emoji_id: product.button_custom_emoji_id,
            description: product.description,
            image_url: product.image_url,
            delivery_type,
            stock_count,
            plans,
        });
    }
    Ok(ok(items))
}

pub async fn get_request_status(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Path(request_id): Path<i64>,
) -> ApiResult<PurchaseRequestResponse> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    let Some(request) = repo::get_purchase_request(&ctx.pool, request_id)
        .await
        .map_err(|e| ApiError::internal(format!("get request failed: {e}")))?
    else {
        return Err(ApiError::not_found("purchase request not found"));
    };
    if request.child_bot_id != auth.child_bot.id {
        return Err(ApiError::not_found("purchase request not found"));
    }
    Ok(ok(PurchaseRequestResponse {
        request_id: request.id,
        status: request.status,
        child_bot_id: request.child_bot_id,
        affiliate_user_id: request.affiliate_user_id,
        telegram_id: request.buyer_user_id,
        product_id: request.product_id,
        qty: request.qty,
        amount: request.amount,
        amount_display: format_vnd(request.amount),
        confirmation_sent: true,
        order_id: request.order_id,
        balance_after: None,
        balance_after_display: None,
        delivered_data: None,
        usage_instructions: None,
    }))
}

pub async fn create_purchase_request(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Json(payload): Json<CreatePurchaseRequestPayload>,
) -> ApiResult<PurchaseRequestResponse> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    let buyer_user_id = payload.telegram_id;
    let order = buy_with_reseller_wallet(&ctx, &auth.child_bot, payload)
        .await
        .map_err(child_bot_request_error)?;
    let delivered_data = if let Some(usage) = order.usage_instructions.as_deref() {
        format!(
            "{}\n\n📘 Hướng dẫn sử dụng\n\n{}",
            order.delivered_data, usage
        )
    } else {
        order.delivered_data.clone()
    };

    Ok(ok(PurchaseRequestResponse {
        request_id: 0,
        status: "paid".to_string(),
        child_bot_id: auth.child_bot.id,
        affiliate_user_id: auth.child_bot.owner_user_id,
        telegram_id: buyer_user_id,
        product_id: order.product_id,
        qty: order.qty,
        amount: order.amount,
        amount_display: format_vnd(order.amount),
        confirmation_sent: false,
        order_id: Some(order.order_id),
        balance_after: Some(order.balance_after),
        balance_after_display: Some(format_vnd(order.balance_after)),
        delivered_data: Some(delivered_data),
        usage_instructions: order.usage_instructions,
    }))
}

fn child_bot_request_error(err: anyhow::Error) -> ApiError {
    let msg = err.to_string();
    if msg.contains("not found") {
        ApiError::not_found(msg)
    } else if msg.contains("telegram_id")
        || msg.contains("không đủ")
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

async fn buy_with_reseller_wallet(
    ctx: &AppContext,
    child_bot: &ChildBotRecord,
    payload: CreatePurchaseRequestPayload,
) -> Result<DirectChildBotOrder> {
    if payload.telegram_id <= 0 {
        return Err(anyhow!("telegram_id is invalid"));
    }
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
    let order_qty = if delivery_type == "manual_input" {
        selected_plan.map(|plan| plan.months).unwrap_or(qty)
    } else {
        qty
    };
    let amount = selected_plan
        .as_ref()
        .map(|plan| plan.price)
        .unwrap_or(product.price * qty);

    let reseller_user_id = child_bot.owner_user_id;
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, reseller_user_id).await?;
    if wallet.balance < amount {
        return Err(anyhow!(
            "Số dư ví CTV không đủ. Hiện có {}, cần {}",
            format_vnd(wallet.balance),
            format_vnd(amount),
        ));
    }

    let mut order = Order::new(
        reseller_user_id,
        reseller_user_id,
        product.id,
        order_qty,
        amount,
        new_childbot_memo(child_bot.id),
        payload.customer_input.clone(),
        selected_plan.as_ref().map(|plan| plan.id),
        selected_plan.as_ref().map(|plan| plan.label.clone()),
        selected_plan.as_ref().map(|plan| plan.months),
        selected_plan.as_ref().map(|plan| plan.price),
    );

    let mut tx = ctx.pool.begin().await?;
    let (delivered_data, reserved_item_ids) = reserve_delivery_data(
        &mut tx,
        &product,
        &delivery_type,
        order_qty,
        payload.customer_input.as_deref(),
    )
    .await?;
    order.delivered_data = Some(delivered_data.clone());
    order.reserved_item_ids = reserved_item_ids;
    orders_repo::insert_order_tx(&mut tx, &order).await?;
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        reseller_user_id,
        amount,
        &order.id,
        Some("childbot_reseller_wallet_purchase"),
    )
    .await?;
    let paid_at = Utc::now();
    orders_repo::mark_order_paid(
        &mut tx,
        &order.id,
        "childbot_reseller_wallet",
        paid_at,
        Some(&delivered_data),
        order.reserved_item_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    order.status = OrderStatus::Paid;
    order.payment_tx_id = Some("childbot_reseller_wallet".to_string());
    order.paid_at = Some(paid_at.to_rfc3339());

    repo::insert_child_bot_order(
        &ctx.pool,
        &order.id,
        child_bot.id,
        reseller_user_id,
        payload.telegram_id,
    )
    .await?;

    let paid_order = OrderWithProduct {
        order: order.clone(),
        product: product.clone(),
    };
    if let Err(err) = notify_admins_order_paid(
        ctx,
        &paid_order,
        "childbot_reseller_wallet",
        paid_at,
        &PaymentSource::ClientApiWallet,
    )
    .await
    {
        tracing::error!("send paid-order admin notification after child bot wallet payment failed: {err}");
    }

    let usage_instructions = usage_instructions::get_usage_instructions(&ctx.pool, product.id)
        .await?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(DirectChildBotOrder {
        order_id: order.id,
        product_id: product.id,
        qty: order_qty,
        amount,
        balance_after,
        delivered_data,
        usage_instructions,
    })
}

async fn reserve_delivery_data(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product: &Product,
    delivery_type: &str,
    qty: i64,
    customer_input: Option<&str>,
) -> Result<(String, Option<String>)> {
    if delivery_type == "manual_input" {
        let info = customer_input
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("N/A");
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

fn new_childbot_memo(child_bot_id: i64) -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>()
        .to_ascii_uppercase();
    format!("CB{child_bot_id}{suffix}")
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/childbot/me", get(me))
        .route("/api/childbot/balance", get(balance))
        .route("/api/childbot/orders", get(list_orders))
        .route("/api/childbot/products", get(list_products))
        .route("/api/childbot/purchase-requests", post(create_purchase_request))
        .route("/api/childbot/purchase-requests/:request_id", get(get_request_status))
}
