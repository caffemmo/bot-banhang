use std::sync::Arc;

use axum::extract::{Query, State};
use serde::Deserialize;

use crate::app::AppContext;
use crate::domains::products::repo;

use crate::core::pagination::normalize_pagination;
use crate::core::responses::{ApiResult, PaginatedResponse, ok};

#[derive(Debug, Deserialize)]
pub struct ListWebhookEventsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub provider: Option<String>,
    pub memo: Option<String>,
    pub tx_id: Option<String>,
}

pub async fn list_webhook_events(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListWebhookEventsQuery>,
) -> ApiResult<PaginatedResponse<repo::WebhookEventListRow>> {
    let (limit, offset) = normalize_pagination(q.limit, q.offset);

    let items = repo::list_webhook_events(
        &ctx.pool,
        limit,
        offset,
        q.provider.as_deref(),
        q.memo.as_deref(),
        q.tx_id.as_deref(),
    )
    .await
    .map_err(|e| crate::core::responses::ApiError::internal(e.to_string()))?;

    let total = repo::count_webhook_events(
        &ctx.pool,
        q.provider.as_deref(),
        q.memo.as_deref(),
        q.tx_id.as_deref(),
    )
    .await
    .map_err(|e| crate::core::responses::ApiError::internal(e.to_string()))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

use axum::Router;
use axum::routing::get;

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new().route("/api/admin/webhooks/events", get(list_webhook_events))
}
