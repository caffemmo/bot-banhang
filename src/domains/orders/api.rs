use std::sync::Arc;

use anyhow::{Result, anyhow};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use teloxide::payloads::{SendDocumentSetters, SendMessageSetters};
use teloxide::requests::Requester;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::domains::orders::fulfillment::{PaymentSource, fulfill_paid_order};
use crate::domains::orders::models::OrderStatus;
use crate::domains::orders::models::OrderWithProduct;
use crate::domains::products::models::Product;
use crate::domains::products::repo;

use crate::core::pagination::normalize_pagination;
use crate::core::responses::{Ack, ApiError, ApiResult, PaginatedResponse, ok};

pub const RESERVE_TTL_MINUTES: i64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadedFileDelivery {
    pub path: String,
    pub name: String,
    pub mime: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListOrdersQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub query: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MarkPaidPayload {
    pub payment_tx_id: String,
    pub paid_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct CancelPayload {
    pub reason: Option<String>,
}

pub async fn list_orders(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Query(params): Query<ListOrdersQuery>,
) -> ApiResult<PaginatedResponse<OrderWithProduct>> {
    let status = parse_status_filter(params.status.as_deref());
    let from = parse_date_filter(params.from, false)?;
    let to = parse_date_filter(params.to, true)?;
    let (limit, offset) = normalize_pagination(params.limit, params.offset);

    let items: Vec<crate::domains::orders::models::OrderWithProduct> = repo::list_orders_admin(
        &ctx.pool,
        limit,
        offset,
        status,
        params.query.as_deref(),
        from.as_deref(),
        to.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("list orders failed: {e}")))?;

    let total = repo::count_orders_admin(
        &ctx.pool,
        status,
        params.query.as_deref(),
        from.as_deref(),
        to.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("count orders failed: {e}")))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn get_order_detail(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(order_id): Path<String>,
) -> ApiResult<OrderWithProduct> {
    let Some(order) = repo::get_order_with_product(&ctx.pool, &order_id)
        .await
        .map_err(|e| ApiError::internal(format!("get order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };

    Ok(ok(order))
}

pub async fn mark_paid_manual(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(order_id): Path<String>,
    Json(payload): Json<MarkPaidPayload>,
) -> ApiResult<OrderWithProduct> {
    let Some(order) = repo::get_order_with_product(&ctx.pool, &order_id)
        .await
        .map_err(|e| ApiError::internal(format!("get order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };

    if payload.payment_tx_id.trim().is_empty() {
        return Err(ApiError::validation("payment_tx_id is required"));
    }

    if matches!(order.order.status, OrderStatus::Paid) {
        return Ok(ok(order));
    }

    if is_order_expired(&order.order.created_at) {
        release_reservation(&ctx, &order, OrderStatus::Expired)
            .await
            .map_err(|e| ApiError::internal(format!("expire order failed: {e}")))?;
        return Err(ApiError::validation(
            "order expired, please create a new one",
        ));
    }

    let paid_at = parse_paid_at(payload.paid_at.as_deref())
        .map_err(|_| ApiError::validation("invalid paid_at"))?;

    fulfill_paid_order(
        ctx.clone(),
        &order_id,
        &payload.payment_tx_id,
        paid_at,
        PaymentSource::AdminManual {
            admin_user_id: None,
        },
    )
    .await
    .map_err(|e| ApiError::internal(format!("fulfill order failed: {e}")))?;

    let Some(order) = repo::get_order_with_product(&ctx.pool, &order_id)
        .await
        .map_err(|e| ApiError::internal(format!("reload order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };

    tracing::info!("order {} -> paid (manual)", order_id);

    Ok(ok(order))
}

pub async fn cancel_order(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(order_id): Path<String>,
    Json(_payload): Json<CancelPayload>,
) -> ApiResult<OrderWithProduct> {
    let Some(mut order) = repo::get_order_with_product(&ctx.pool, &order_id)
        .await
        .map_err(|e| ApiError::internal(format!("get order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };

    if matches!(order.order.status, OrderStatus::Paid) {
        return Err(ApiError::validation("cannot cancel a paid order"));
    }

    let mut tx = ctx
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(ids_str) = &order.order.reserved_item_ids {
        let ids = parse_reserved_ids(ids_str);
        if !ids.is_empty() {
            repo::return_product_items(&mut tx, order.order.product_id, &ids)
                .await
                .map_err(|e| ApiError::internal(format!("return items failed: {e}")))?;
        }
    }
    repo::update_order_status_with_data(&mut tx, &order_id, OrderStatus::Cancel, None, None)
        .await
        .map_err(|e| ApiError::internal(format!("update order failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(format!("commit failed: {e}")))?;

    tracing::info!("order {} -> cancel (admin)", order_id);

    order.order.status = OrderStatus::Cancel;
    order.order.delivered_data = None;
    order.order.reserved_item_ids = None;
    Ok(ok(order))
}

pub async fn resend_data(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(order_id): Path<String>,
) -> ApiResult<Ack> {
    let Some(order) = repo::get_order_with_product(&ctx.pool, &order_id)
        .await
        .map_err(|e| ApiError::internal(format!("get order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };

    if !matches!(order.order.status, OrderStatus::Paid) {
        return Err(ApiError::validation("order is not paid"));
    }

    let delivered = order
        .order
        .delivered_data
        .clone()
        .ok_or_else(|| ApiError::validation("no delivered data to resend"))?;

    send_product_file(&ctx, &order, &delivered)
        .await
        .map_err(|e| ApiError::internal(format!("resend failed: {e}")))?;

    Ok(ok(Ack { success: true }))
}

fn status_str(status: &OrderStatus) -> &'static str {
    match status {
        OrderStatus::Pending => "pending",
        OrderStatus::Paid => "paid",
        OrderStatus::Refunded => "refunded",
        OrderStatus::Cancel => "cancel",
        OrderStatus::Expired => "expired",
    }
}

pub async fn export_orders(
    State(ctx): State<Arc<AppContext>>,
    __headers: axum::http::HeaderMap,
    Query(params): Query<ListOrdersQuery>,
) -> impl IntoResponse {
    let status = parse_status_filter(params.status.as_deref());
    let from = parse_date_filter(params.from, false).unwrap_or(None);
    let to = parse_date_filter(params.to, true).unwrap_or(None);
    // Export up to 10k rows to keep response reasonable.
    let limit = 10_000;
    let orders = match repo::list_orders_admin(
        &ctx.pool,
        limit,
        params.offset.unwrap_or(0),
        status,
        params.query.as_deref(),
        from.as_deref(),
        to.as_deref(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("export failed: {e}"),
            )
                .into_response();
        }
    };

    let mut wtr = String::new();
    wtr.push_str(
        "id,created_at,status,amount,bank_memo,user_id,product,qty,plan_label,customer_input\n",
    );
    fn esc(s: &str) -> String {
        s.replace('"', "'")
    }
    for o in orders {
        let row = format!(
            "\"{}\",{}, {}, {},\"{}\",{},\"{}\",{},\"{}\",\"{}\"\n",
            o.order.id,
            o.order.created_at,
            status_str(&o.order.status),
            o.order.amount,
            esc(&o.order.bank_memo),
            o.order.user_id,
            esc(&o.product.name),
            o.order.qty,
            esc(&o.order.plan_label.unwrap_or_default()),
            esc(&o.order.customer_input.unwrap_or_default())
        );
        wtr.push_str(&row);
    }

    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/csv"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"orders.csv\"",
            ),
        ],
        wtr,
    )
        .into_response()
}

pub fn parse_paid_at(raw: Option<&str>) -> Result<DateTime<Utc>> {
    match raw {
        Some(ts) => {
            // 1) RFC3339 (legacy/test UI)
            if let Ok(dt) = ts.parse::<DateTime<Utc>>() {
                return Ok(dt);
            }

            // 2) SePay: "YYYY-MM-DD HH:mm:ss" (typically local time)
            if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
                // SePay timestamps are usually Vietnam time (UTC+7).
                let vn = chrono::FixedOffset::east_opt(7 * 3600).unwrap();
                let dt = vn
                    .from_local_datetime(&naive)
                    .single()
                    .unwrap_or_else(|| vn.from_local_datetime(&naive).earliest().unwrap());
                return Ok(dt.with_timezone(&Utc));
            }

            Err(anyhow!("invalid paid_at"))
        }
        None => Ok(Utc::now()),
    }
}

pub fn is_order_expired(created_at: &str) -> bool {
    if let Ok(dt) = DateTime::parse_from_rfc3339(created_at) {
        let elapsed = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
        elapsed.num_minutes() >= RESERVE_TTL_MINUTES
    } else {
        false
    }
}

pub async fn release_reservation(
    ctx: &AppContext,
    order: &OrderWithProduct,
    status: OrderStatus,
) -> Result<()> {
    let mut tx = ctx.pool.begin().await?;
    if let Some(ids_str) = &order.order.reserved_item_ids {
        let ids = parse_reserved_ids(ids_str);
        if !ids.is_empty() {
            repo::return_product_items(&mut tx, order.order.product_id, &ids).await?;
        }
    }
    repo::update_order_status_with_data(&mut tx, &order.order.id, status, None, None).await?;
    tx.commit().await?;
    tracing::info!(
        "order {} -> {} (release)",
        order.order.id,
        status.to_string()
    );
    Ok(())
}

pub async fn send_product_file(
    ctx: &AppContext,
    owp: &OrderWithProduct,
    delivered_data: &str,
) -> Result<()> {
    use teloxide::types::InputFile;

    let lang = i18n::user_lang_by_id(ctx, owp.order.user_id).await;
    let continue_shopping_btn =
        i18n::t(ctx, &lang, "continue_shopping_btn", "🛒 Continue shopping");

    if owp.product.category.as_deref() == Some("Viameta") {
        ctx.bot
            .send_message(ChatId(owp.order.chat_id), delivered_data.to_string())
            .reply_markup(InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "⚡ Dịch vụ tích xanh",
                    "viameta:menu",
                )],
                vec![InlineKeyboardButton::callback(
                    continue_shopping_btn,
                    "start:shop",
                )],
            ]))
            .await?;
        return Ok(());
    }

    if product_delivery_type(&owp.product) == "uploaded_file" {
        let uploaded_items = parse_uploaded_file_delivery_items(delivered_data)?;
        if !uploaded_items.is_empty() {
            let caption = i18n::tr(
                ctx,
                &lang,
                "delivery_uploaded_files_caption",
                "✅ Payment successful {memo}! ❤️\n\nProduct: {product}\nOrder ID: {order_id}\nQuantity: {qty}\nAmount: {amount} VND\n\nProduct files are sent below.",
                &[
                    ("memo", owp.order.bank_memo.clone()),
                    ("product", display_product_name(ctx, &owp.product.name)),
                    ("order_id", owp.order.id.clone()),
                    ("qty", owp.order.qty.to_string()),
                    ("amount", owp.order.amount.to_string()),
                ],
            );
            let caption_rich =
                i18n::rich_text_for_key(ctx, "delivery_uploaded_files_caption", caption);

            for (index, item) in uploaded_items.iter().enumerate() {
                tokio::fs::metadata(&item.path)
                    .await
                    .map_err(|e| anyhow!("uploaded product file not found: {}: {e}", item.path))?;

                let mut request = ctx.bot.send_document(
                    ChatId(owp.order.chat_id),
                    InputFile::file(item.path.clone()).file_name(item.name.clone()),
                );
                if index == 0 {
                    request = request.caption(caption_rich.text.clone());
                    if !caption_rich.entities.is_empty() {
                        request = request.caption_entities(caption_rich.entities.clone());
                    }
                    request = request.reply_markup(InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback(continue_shopping_btn.clone(), "start:shop"),
                    ]]));
                }
                request.await?;
            }

            return Ok(());
        }

        let file_path = owp
            .product
            .file_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("uploaded-file product missing file_path"))?;
        let file_name = owp
            .product
            .file_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("product-file");

        tokio::fs::metadata(file_path)
            .await
            .map_err(|e| anyhow!("uploaded product file not found: {file_path}: {e}"))?;

        let caption = i18n::tr(
            ctx,
            &lang,
            "delivery_uploaded_file_caption",
            "✅ Payment successful {memo}! ❤️\n\nProduct: {product}\nOrder ID: {order_id}\nQuantity: {qty}\nAmount: {amount} VND\nFile: {file_name}\n\nProduct file is sent below.",
            &[
                ("memo", owp.order.bank_memo.clone()),
                ("product", display_product_name(ctx, &owp.product.name)),
                ("order_id", owp.order.id.clone()),
                ("qty", owp.order.qty.to_string()),
                ("amount", owp.order.amount.to_string()),
                ("file_name", file_name.to_string()),
            ],
        );
        let caption_rich = i18n::rich_text_for_key(ctx, "delivery_uploaded_file_caption", caption);
        let mut request = ctx
            .bot
            .send_document(
                ChatId(owp.order.chat_id),
                InputFile::file(file_path.to_string()).file_name(file_name.to_string()),
            )
            .caption(caption_rich.text)
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback(continue_shopping_btn, "start:shop"),
            ]]));
        if !caption_rich.entities.is_empty() {
            request = request.caption_entities(caption_rich.entities);
        }
        request.await?;

        return Ok(());
    }

    let plan_label = owp
        .order
        .plan_label
        .clone()
        .unwrap_or_else(|| i18n::t(ctx, &lang, "delivery_plan_none", "None"));
    let customer_input = owp
        .order
        .customer_input
        .clone()
        .unwrap_or_else(|| i18n::t(ctx, &lang, "delivery_customer_none", "Not provided"));
    let content = i18n::tr(
        ctx,
        &lang,
        "delivery_text_file_content",
        "Paid order\n----------------------\nOrder ID : {order_id}\nMemo     : {memo}\nProduct  : {product}\nPlan     : {plan}\nQuantity : {qty}\nAmount   : {amount} VND\nCustomer info: {customer}\n\nAttached data:\n{data}",
        &[
            ("order_id", owp.order.id.clone()),
            ("memo", owp.order.bank_memo.clone()),
            ("product", display_product_name(ctx, &owp.product.name)),
            ("plan", plan_label),
            ("qty", owp.order.qty.to_string()),
            ("amount", owp.order.amount.to_string()),
            ("customer", customer_input),
            ("data", delivered_data.to_string()),
        ],
    );

    ctx.bot
        .send_document(
            ChatId(owp.order.chat_id),
            InputFile::memory(content.into_bytes())
                .file_name(format!("data_{}.txt", owp.order.bank_memo)),
        )
        .caption(i18n::tr(
            ctx,
            &lang,
            "delivery_data_file_caption",
            "✅ Payment successful {memo}! ❤️\n\nAccount data is sent in the file below.",
            &[("memo", owp.order.bank_memo.clone())],
        ))
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(continue_shopping_btn, "start:shop"),
        ]]))
        .await?;

    Ok(())
}

fn display_product_name(ctx: &AppContext, name: &str) -> String {
    i18n::rich_text_for_key(ctx, "", name).text
}

pub fn product_delivery_type(product: &Product) -> &str {
    product
        .delivery_type
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(if product.requires_input.unwrap_or(0) == 1 {
            "manual_input"
        } else {
            "stock_item"
        })
}

pub fn uploaded_file_marker(product_id: i64) -> String {
    format!("uploaded_file:{product_id}")
}

pub fn uploaded_file_delivery_payload(
    path: &str,
    name: &str,
    mime: Option<&str>,
) -> Result<String> {
    let path = path.trim();
    let name = name.trim();
    if path.is_empty() || name.is_empty() {
        return Err(anyhow!("uploaded file path and name are required"));
    }

    serde_json::to_string(&UploadedFileDelivery {
        path: path.to_string(),
        name: name.to_string(),
        mime: mime.map(|s| s.to_string()).filter(|s| !s.trim().is_empty()),
    })
    .map_err(Into::into)
}

pub fn parse_uploaded_file_delivery_items(
    delivered_data: &str,
) -> Result<Vec<UploadedFileDelivery>> {
    delivered_data
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("uploaded_file:"))
        .map(|line| serde_json::from_str::<UploadedFileDelivery>(line).map_err(Into::into))
        .collect()
}

pub(crate) fn parse_reserved_ids(data: &str) -> Vec<i64> {
    data.split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect()
}

fn parse_status_filter(raw: Option<&str>) -> Option<OrderStatus> {
    match raw {
        Some("pending") => Some(OrderStatus::Pending),
        Some("paid") => Some(OrderStatus::Paid),
        Some("refunded") => Some(OrderStatus::Refunded),
        Some("cancel") => Some(OrderStatus::Cancel),
        Some("expired") => Some(OrderStatus::Expired),
        _ => None,
    }
}

fn parse_date_filter(raw: Option<String>, end_of_day: bool) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    if let Ok(dt) = DateTime::parse_from_rfc3339(&raw) {
        return Ok(Some(dt.with_timezone(&Utc).to_rfc3339()));
    }

    if let Ok(date) = NaiveDate::parse_from_str(&raw, "%Y-%m-%d") {
        let time = if end_of_day {
            date.and_hms_milli_opt(23, 59, 59, 999)
        } else {
            date.and_hms_opt(0, 0, 0)
        }
        .ok_or_else(|| ApiError::validation("invalid date"))?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(time, Utc);
        return Ok(Some(dt.to_rfc3339()));
    }

    Err(ApiError::validation("invalid date filter"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploaded_file_delivery_items_parse_from_reserved_stock_payloads() {
        let delivered = [
            uploaded_file_delivery_payload(
                "storage/product_files/a.zip",
                "a.zip",
                Some("application/zip"),
            )
            .unwrap(),
            uploaded_file_delivery_payload("storage/product_files/b.pdf", "b.pdf", None).unwrap(),
        ]
        .join("\n");

        let files = parse_uploaded_file_delivery_items(&delivered).unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "storage/product_files/a.zip");
        assert_eq!(files[0].name, "a.zip");
        assert_eq!(files[0].mime.as_deref(), Some("application/zip"));
        assert_eq!(files[1].path, "storage/product_files/b.pdf");
        assert_eq!(files[1].name, "b.pdf");
        assert_eq!(files[1].mime, None);
    }
}

use axum::Router;
use axum::routing::{get, post};

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/api/admin/orders", get(list_orders))
        .route("/api/admin/orders/export", get(export_orders))
        .route("/api/admin/orders/:id", get(get_order_detail))
        .route("/api/admin/orders/:id/mark_paid", post(mark_paid_manual))
        .route("/api/admin/orders/:id/cancel", post(cancel_order))
        .route("/api/admin/orders/:id/resend", post(resend_data))
}
