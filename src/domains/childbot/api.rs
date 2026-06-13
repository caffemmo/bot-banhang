use std::sync::Arc;

use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::AppContext;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::orders::api as orders_api;
use crate::domains::products::models::ProductPlan;
use crate::domains::products::repo as products_repo;

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
pub struct ChildBotProductItem {
    pub id: i64,
    pub name: String,
    pub price: i64,
    pub category: Option<String>,
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
    }))
}

pub async fn create_purchase_request(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Json(payload): Json<CreatePurchaseRequestPayload>,
) -> ApiResult<PurchaseRequestResponse> {
    let auth = require_child_bot_auth(&ctx, &headers).await?;
    let request = create_pending_request(&ctx, &auth.child_bot, payload)
        .await
        .map_err(child_bot_request_error)?;
    send_confirmation_to_buyer(&ctx, &auth.child_bot, &request)
        .await
        .map_err(|e| ApiError::validation(format!("Không gửi được xác nhận cho khách. Khách cần bấm /start ở bot chính trước. Chi tiết: {e}")))?;

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
    }))
}

fn child_bot_request_error(err: anyhow::Error) -> ApiError {
    let msg = err.to_string();
    if msg.contains("not found") {
        ApiError::not_found(msg)
    } else if msg.contains("telegram_id")
        || msg.contains("qty")
        || msg.contains("plan")
        || msg.contains("stock")
    {
        ApiError::validation(msg)
    } else {
        ApiError::internal(msg)
    }
}

async fn create_pending_request(
    ctx: &AppContext,
    child_bot: &ChildBotRecord,
    payload: CreatePurchaseRequestPayload,
) -> Result<repo::ChildBotPurchaseRequest> {
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
    if delivery_type != "manual_input" {
        let stock_count = products_repo::count_product_items(&ctx.pool, product.id).await?;
        if stock_count < order_qty {
            return Err(anyhow!("stock is not enough"));
        }
    }
    let amount = selected_plan
        .as_ref()
        .map(|plan| plan.price)
        .unwrap_or(product.price * qty);

    repo::create_purchase_request(
        &ctx.pool,
        child_bot.id,
        child_bot.owner_user_id,
        payload.telegram_id,
        payload.chat_id.unwrap_or(payload.telegram_id),
        product.id,
        order_qty,
        selected_plan.map(|plan| plan.id),
        payload.customer_input.as_deref(),
        amount,
    )
    .await
}

async fn send_confirmation_to_buyer(
    ctx: &AppContext,
    child_bot: &ChildBotRecord,
    request: &repo::ChildBotPurchaseRequest,
) -> Result<()> {
    let product = products_repo::get_product(&ctx.pool, request.product_id)
        .await?
        .ok_or_else(|| anyhow!("product not found"))?;
    let shop_name = child_bot
        .shop_name
        .clone()
        .or_else(|| child_bot.bot_username.clone())
        .unwrap_or_else(|| format!("Bot con #{}", child_bot.id));
    let text = format!(
        "🤖 Xác nhận mua hàng từ bot con\n\nShop: {}\nSản phẩm: {}\nSố lượng: {}\nSố tiền: {}\n\nBấm xác nhận nếu đúng là bạn đang mua đơn này. Bot chính chỉ trừ ví sau khi bạn bấm xác nhận.",
        shop_name,
        product.name,
        request.qty,
        format_vnd(request.amount),
    );
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "✅ Xác nhận mua",
            format!("childbot_order:confirm:{}:{}", request.id, request.confirm_token),
        ),
        InlineKeyboardButton::callback(
            "❌ Hủy",
            format!("childbot_order:cancel:{}:{}", request.id, request.confirm_token),
        ),
    ]]);
    ctx.bot
        .send_message(ChatId(request.buyer_chat_id), text)
        .reply_markup(keyboard)
        .await?;
    Ok(())
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

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/childbot/me", get(me))
        .route("/api/childbot/products", get(list_products))
        .route("/api/childbot/purchase-requests", post(create_purchase_request))
        .route("/api/childbot/purchase-requests/:request_id", get(get_request_status))
}
