use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use teloxide::requests::Requester;
use teloxide::types::ChatId;

use crate::app::AppContext;
use crate::core::pagination::normalize_pagination;
use crate::core::responses::{Ack, ApiError, ApiResult, PaginatedResponse, ok};
use crate::domains::chat::repo;

#[derive(Debug, Deserialize)]
pub struct ConversationsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub query: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub chat_id: i64,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessagePayload {
    pub chat_id: i64,
    pub text: String,
}

pub async fn list_conversations(
    State(ctx): State<Arc<AppContext>>,
    Query(params): Query<ConversationsQuery>,
) -> ApiResult<PaginatedResponse<crate::domains::chat::models::Conversation>> {
    let (limit, offset) = normalize_pagination(params.limit, params.offset);
    let items = repo::list_conversations(&ctx.pool, limit, offset, params.query.as_deref())
        .await
        .map_err(|e| ApiError::internal(format!("list conversations failed: {e}")))?;
    let total = repo::count_conversations(&ctx.pool, params.query.as_deref())
        .await
        .map_err(|e| ApiError::internal(format!("count conversations failed: {e}")))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn list_messages(
    State(ctx): State<Arc<AppContext>>,
    Query(params): Query<MessagesQuery>,
) -> ApiResult<PaginatedResponse<crate::domains::chat::models::ChatMessage>> {
    let (limit, offset) = normalize_pagination(params.limit, params.offset);
    let items = repo::list_messages(&ctx.pool, params.chat_id, limit, offset)
        .await
        .map_err(|e| ApiError::internal(format!("list messages failed: {e}")))?;
    let total = repo::count_messages(&ctx.pool, params.chat_id)
        .await
        .map_err(|e| ApiError::internal(format!("count messages failed: {e}")))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn send_message(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<SendMessagePayload>,
) -> ApiResult<Ack> {
    let text = payload.text.trim();
    if text.is_empty() {
        return Err(ApiError::validation("text is required"));
    }

    let sent = ctx
        .bot
        .send_message(ChatId(payload.chat_id), text.to_string())
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "TELEGRAM_SEND_FAILED",
                format!("telegram send failed: {e}"),
            )
        })?;

    let raw_json = serde_json::to_value(&sent).ok();
    let telegram_date = Some(sent.date.to_rfc3339());

    repo::insert_message(
        &ctx.pool,
        payload.chat_id,
        None,
        "out",
        Some(text),
        Some(sent.id.0 as i64),
        telegram_date.as_deref(),
        raw_json.as_ref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("save outgoing message failed: {e}")))?;

    if let Some(raw) = raw_json.as_ref() {
        let _ = repo::insert_update_log(&ctx.pool, Some(payload.chat_id), None, "admin_send", raw)
            .await;
    }

    Ok(ok(Ack { success: true }))
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/admin/chat/conversations", get(list_conversations))
        .route(
            "/api/admin/chat/messages",
            get(list_messages).post(send_message),
        )
}
