use std::collections::BTreeSet;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};
use uuid::Uuid;

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::support::models::{SupportMessage, SupportTicket, support_kind_label};
use crate::domains::support::repo::{self, NewSupportTicket};

#[derive(Debug, Deserialize)]
pub struct CreateTicketPayload {
    pub kind: String,
    pub customer_name: Option<String>,
    pub contact_method: String,
    pub contact_value: Option<String>,
    pub order_ref: Option<String>,
    pub facebook_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct TicketKeyQuery {
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct CustomerMessagePayload {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct TicketResponse {
    pub ticket: SupportTicket,
    pub messages: Vec<SupportMessage>,
    pub ticket_url: String,
}

pub async fn create_ticket(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<CreateTicketPayload>,
) -> ApiResult<TicketResponse> {
    let kind = normalize_kind(&payload.kind)?;
    let contact_method = normalize_contact_method(&payload.contact_method)?;
    let message = clean_required(&payload.message, "message")?;
    let customer_name = clean_optional(payload.customer_name.as_deref());
    let contact_value = clean_optional(payload.contact_value.as_deref());
    let order_ref = clean_optional(payload.order_ref.as_deref());
    let facebook_ref = clean_optional(payload.facebook_ref.as_deref());
    let public_key = Uuid::new_v4().simple().to_string();

    let ticket = repo::create_ticket(
        &ctx.pool,
        NewSupportTicket {
            public_key: &public_key,
            kind,
            customer_name: customer_name.as_deref(),
            contact_method,
            contact_value: contact_value.as_deref(),
            order_ref: order_ref.as_deref(),
            facebook_ref: facebook_ref.as_deref(),
            message: &message,
        },
    )
    .await
    .map_err(|e| ApiError::internal(format!("create support ticket failed: {e}")))?;

    let messages = repo::list_messages(&ctx.pool, ticket.id)
        .await
        .map_err(|e| ApiError::internal(format!("list support messages failed: {e}")))?;
    let ticket_url = ticket_url(&ctx, &ticket);
    notify_admins_ticket_created(&ctx, &ticket, &message, &ticket_url).await;

    Ok(ok(TicketResponse {
        ticket,
        messages,
        ticket_url,
    }))
}

pub async fn get_ticket(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Query(query): Query<TicketKeyQuery>,
) -> ApiResult<TicketResponse> {
    let ticket = repo::get_ticket_by_public_key(&ctx.pool, id, query.key.trim())
        .await
        .map_err(|e| ApiError::internal(format!("get support ticket failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("ticket not found"))?;
    let messages = repo::list_messages(&ctx.pool, ticket.id)
        .await
        .map_err(|e| ApiError::internal(format!("list support messages failed: {e}")))?;
    let ticket_url = ticket_url(&ctx, &ticket);
    Ok(ok(TicketResponse {
        ticket,
        messages,
        ticket_url,
    }))
}

pub async fn add_customer_message(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Query(query): Query<TicketKeyQuery>,
    Json(payload): Json<CustomerMessagePayload>,
) -> ApiResult<TicketResponse> {
    let ticket = repo::get_ticket_by_public_key(&ctx.pool, id, query.key.trim())
        .await
        .map_err(|e| ApiError::internal(format!("get support ticket failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("ticket not found"))?;
    if ticket.status == "closed" {
        return Err(ApiError::validation("ticket is closed"));
    }

    let message = clean_required(&payload.message, "message")?;
    repo::add_message(&ctx.pool, ticket.id, "customer", &message)
        .await
        .map_err(|e| ApiError::internal(format!("add support message failed: {e}")))?;

    let ticket = repo::get_ticket(&ctx.pool, ticket.id)
        .await
        .map_err(|e| ApiError::internal(format!("reload support ticket failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("ticket not found"))?;
    let messages = repo::list_messages(&ctx.pool, ticket.id)
        .await
        .map_err(|e| ApiError::internal(format!("list support messages failed: {e}")))?;
    let ticket_url = ticket_url(&ctx, &ticket);
    notify_admins_customer_message(&ctx, &ticket, &message, &ticket_url).await;

    Ok(ok(TicketResponse {
        ticket,
        messages,
        ticket_url,
    }))
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/support/tickets", post(create_ticket))
        .route("/api/support/tickets/:id", get(get_ticket))
        .route("/api/support/tickets/:id/messages", post(add_customer_message))
}

fn normalize_kind(kind: &str) -> Result<&'static str, ApiError> {
    match kind.trim() {
        "order" => Ok("order"),
        "meta_verified" => Ok("meta_verified"),
        "facebook_unlock" => Ok("facebook_unlock"),
        _ => Err(ApiError::validation("kind is invalid")),
    }
}

fn normalize_contact_method(method: &str) -> Result<&'static str, ApiError> {
    match method.trim() {
        "web" => Ok("web"),
        "telegram" => Ok("telegram"),
        "zalo" => Ok("zalo"),
        _ => Err(ApiError::validation("contact_method is invalid")),
    }
}

fn clean_required(value: &str, field: &'static str) -> Result<String, ApiError> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return Err(ApiError::validation(format!("{field} is required")));
    }
    Ok(cleaned.chars().take(4000).collect())
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(500).collect())
}

fn ticket_url(ctx: &AppContext, ticket: &SupportTicket) -> String {
    let path = format!("/support?ticket={}&key={}", ticket.id, ticket.public_key);
    ctx.config
        .base_url
        .as_deref()
        .map(|base| format!("{}{}", base.trim_end_matches('/'), path))
        .unwrap_or(path)
}

fn admin_ids(ctx: &AppContext) -> Vec<i64> {
    let mut ids = BTreeSet::new();
    ids.extend(ctx.telegram_icon_admin_ids());
    ids.extend(ctx.order_notification_admin_ids());
    ids.into_iter().collect()
}

async fn notify_admins_ticket_created(
    ctx: &AppContext,
    ticket: &SupportTicket,
    first_message: &str,
    ticket_url: &str,
) {
    let text = format!(
        "🆘 Ticket hỗ trợ mới #{}\nLoại: {}\nKhách: {}\nLiên hệ: {} {}\nMã đơn: {}\nFacebook: {}\n\nNội dung:\n{}\n\nWeb: {}",
        ticket.id,
        support_kind_label(&ticket.kind),
        ticket.customer_name.as_deref().unwrap_or("-"),
        ticket.contact_method,
        ticket.contact_value.as_deref().unwrap_or("-"),
        ticket.order_ref.as_deref().unwrap_or("-"),
        ticket.facebook_ref.as_deref().unwrap_or("-"),
        first_message,
        ticket_url,
    );
    notify_admins(ctx, ticket.id, text).await;
}

async fn notify_admins_customer_message(
    ctx: &AppContext,
    ticket: &SupportTicket,
    message: &str,
    ticket_url: &str,
) {
    let text = format!(
        "💬 Khách nhắn thêm ticket #{}\nLoại: {}\n\n{}\n\nWeb: {}",
        ticket.id,
        support_kind_label(&ticket.kind),
        message,
        ticket_url,
    );
    notify_admins(ctx, ticket.id, text).await;
}

async fn notify_admins(ctx: &AppContext, ticket_id: i64, text: String) {
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✍️ Trả lời", format!("support:reply:{ticket_id}")),
        InlineKeyboardButton::callback("✅ Đóng", format!("support:close:{ticket_id}")),
    ]]);
    for admin_id in admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(ChatId(admin_id), text.clone())
            .reply_markup(keyboard.clone())
            .await;
    }
}
