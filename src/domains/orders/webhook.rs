use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use tracing::warn;

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::domains::orders::models::{OrderStatus, OrderWithProduct};
use crate::domains::products::repo;
use crate::domains::wallet::repo as wallet_repo;

use crate::core::responses::MessageResponse;
use crate::core::time::format_vietnam_datetime;
use crate::domains::orders::api::{is_order_expired, parse_paid_at};
use crate::domains::orders::fulfillment::{PaymentSource, fulfill_paid_order};

pub const TOPUP_TTL_MINUTES: i64 = 30;

#[derive(Debug, Deserialize, Serialize)]
pub struct PaymentWebhook {
    pub memo: String,
    pub amount: i64,
    pub status: String,
    pub tx_id: String,
    pub paid_at: Option<String>,
}

/// SePay webhook payload
/// Example:
/// {
///   "id": 92704,
///   "gateway": "Vietcombank",
///   "transactionDate": "2023-03-25 14:02:37",
///   "accountNumber": "0123499999",
///   "content": "chuyen tien mua iphone",
///   "transferType": "in",
///   "transferAmount": 2277000,
///   "referenceCode": "MBVCB.3278907687"
/// }
#[derive(Debug, Deserialize, Serialize)]
pub struct SePayWebhook {
    pub id: i64,
    #[serde(default)]
    pub gateway: Option<String>,
    #[serde(rename = "transactionDate")]
    pub transaction_date: Option<String>,
    #[serde(rename = "accountNumber")]
    pub account_number: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(rename = "transferType")]
    pub transfer_type: String,
    #[serde(rename = "transferAmount")]
    pub transfer_amount: i64,
    #[serde(default)]
    pub accumulated: Option<i64>,
    #[serde(rename = "subAccount")]
    pub sub_account: Option<String>,
    #[serde(rename = "referenceCode")]
    pub reference_code: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug)]
struct NormalizedPayment {
    memo: String,
    amount: i64,
    status: String,
    tx_id: String,
    paid_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IncomingWebhook {
    Legacy(PaymentWebhook),
    SePay(SePayWebhook),
}

pub async fn handle_webhook(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Json(payload): Json<IncomingWebhook>,
) -> Result<(StatusCode, Json<MessageResponse>), (StatusCode, String)> {
    let provider = match &payload {
        IncomingWebhook::Legacy(_) => "legacy",
        IncomingWebhook::SePay(_) => "sepay",
    };
    let source_ip = extract_source_ip(&headers);
    let raw_json = serde_json::to_string(&payload).ok();

    let authorized = authorize_webhook(&ctx, &headers);
    if !authorized {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            false,
            source_ip.as_deref(),
            None,
            None,
            None,
            None,
            None,
            Some("rejected"),
            Some("unauthorized"),
            raw_json.as_deref(),
        )
        .await;
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".to_string()));
    }

    let payload = normalize_payload(payload, &ctx);

    // SePay can send outgoing transfers too; we treat non-in as no-op.
    let status_lc = payload.status.to_lowercase();
    if status_lc != "paid" {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip.as_deref(),
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("ignored"),
            Some("status not paid"),
            raw_json.as_deref(),
        )
        .await;

        if status_lc == "ignored" {
            return Ok((
                StatusCode::OK,
                Json(MessageResponse {
                    ok: true,
                    message: "ignored".to_string(),
                }),
            ));
        }

        return Err((StatusCode::BAD_REQUEST, "status must be paid".to_string()));
    }

    // NAP prefix → yêu cầu nạp tiền ví
    if payload.memo.starts_with("NAP") {
        return handle_topup_webhook(
            &ctx,
            &payload,
            provider,
            source_ip.as_deref(),
            raw_json.as_deref(),
        )
        .await;
    }

    let Some(order_with_product) = repo::find_order_by_memo(&ctx.pool, &payload.memo)
        .await
        .map_err(internal_error)?
    else {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip.as_deref(),
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("rejected"),
            Some("memo not found"),
            raw_json.as_deref(),
        )
        .await;
        return Err((StatusCode::BAD_REQUEST, "memo not found".to_string()));
    };

    if payload.amount < order_with_product.order.amount {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip.as_deref(),
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            Some(&order_with_product.order.id),
            Some("rejected"),
            Some("amount less than order total"),
            raw_json.as_deref(),
        )
        .await;
        return Err((
            StatusCode::BAD_REQUEST,
            "amount less than order total".to_string(),
        ));
    }

    if matches!(order_with_product.order.status, OrderStatus::Paid) {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip.as_deref(),
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            Some(&order_with_product.order.id),
            Some("ok"),
            Some("already paid"),
            raw_json.as_deref(),
        )
        .await;
        return Ok((
            StatusCode::OK,
            Json(MessageResponse {
                ok: true,
                message: "already paid".to_string(),
            }),
        ));
    }

    if !matches!(order_with_product.order.status, OrderStatus::Pending) {
        return credit_order_payment_to_wallet_response(
            &ctx,
            &order_with_product,
            &payload,
            provider,
            source_ip.as_deref(),
            raw_json.as_deref(),
            order_with_product.order.status,
            "order is not pending",
        )
        .await;
    }

    if is_order_expired(&order_with_product.order.created_at) {
        return credit_order_payment_to_wallet_response(
            &ctx,
            &order_with_product,
            &payload,
            provider,
            source_ip.as_deref(),
            raw_json.as_deref(),
            OrderStatus::Expired,
            "order expired",
        )
        .await;
    }

    let paid_at = parse_paid_at(payload.paid_at.as_deref()).unwrap_or_else(|_| chrono::Utc::now());
    fulfill_paid_order(
        ctx.clone(),
        &order_with_product.order.id,
        &payload.tx_id,
        paid_at,
        PaymentSource::BankWebhook {
            amount_vnd: payload.amount,
        },
    )
    .await
    .map_err(internal_error)?;

    let _ = repo::insert_webhook_event(
        &ctx.pool,
        provider,
        true,
        source_ip.as_deref(),
        Some(&payload.memo),
        Some(&payload.tx_id),
        Some(payload.amount),
        Some(&payload.status),
        Some(&order_with_product.order.id),
        Some("ok"),
        None,
        raw_json.as_deref(),
    )
    .await;

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            ok: true,
            message: "processed".to_string(),
        }),
    ))
}

fn normalize_payload(payload: IncomingWebhook, ctx: &AppContext) -> NormalizedPayment {
    match payload {
        IncomingWebhook::Legacy(p) => NormalizedPayment {
            memo: normalize_payment_memo(&p.memo),
            amount: p.amount,
            status: p.status,
            tx_id: p.tx_id,
            paid_at: p.paid_at,
        },
        IncomingWebhook::SePay(p) => {
            // SePay does not provide our internal memo field directly.
            // We match by memo contained in the transfer content (or SePay "code" when configured).
            // Convention: bot generates a memo like "DHXXXXXXXX" and user transfers with that in content.
            let transfer_content = if p.content.trim().is_empty() {
                p.description.as_deref().unwrap_or("")
            } else {
                &p.content
            };
            let memo = if p.code.as_deref().unwrap_or("").trim().is_empty() {
                extract_memo_from_text(
                    transfer_content,
                    &ctx.order_memo_prefix(),
                    ctx.order_memo_length(),
                )
                .unwrap_or_else(|| normalize_payment_memo(transfer_content))
            } else {
                normalize_payment_memo(&p.code.unwrap())
            };

            let status = if p.transfer_type.to_lowercase() == "in" {
                "paid".to_string()
            } else {
                // ignore outgoing transfers
                "ignored".to_string()
            };

            let tx_id = p
                .reference_code
                .clone()
                .unwrap_or_else(|| format!("sepay:{}", p.id));

            NormalizedPayment {
                memo,
                amount: p.transfer_amount,
                status,
                tx_id,
                paid_at: p.transaction_date,
            }
        }
    }
}

fn normalize_payment_memo(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn extract_memo_from_text(
    text: &str,
    order_prefix: &str,
    order_suffix_len: usize,
) -> Option<String> {
    // Memo format generated by bot: configurable order prefix + random alphanumeric suffix.
    // Keep legacy DH+8 extraction so existing pending orders can still be reconciled.
    // We search case-insensitively within the bank transfer content.
    let s = text.trim();
    if s.is_empty() {
        return None;
    }

    let upper = s.to_uppercase();
    let bytes = upper.as_bytes();
    let is_alnum = |b: u8| b.is_ascii_alphanumeric();
    let normalized_order_prefix = order_prefix.trim().to_ascii_uppercase();
    let mut memo_formats: Vec<(Vec<u8>, usize)> = vec![
        (b"NAP".to_vec(), 8),
        (
            normalized_order_prefix.as_bytes().to_vec(),
            order_suffix_len,
        ),
    ];
    if normalized_order_prefix != "DH" || order_suffix_len != 8 {
        memo_formats.push((b"DH".to_vec(), 8));
    }

    for i in 0..bytes.len() {
        if i > 0 && is_alnum(bytes[i - 1]) {
            continue;
        }
        for (prefix, suffix_len) in &memo_formats {
            let end_prefix = i + prefix.len();
            if prefix.is_empty() || end_prefix > bytes.len() || &bytes[i..end_prefix] != prefix {
                continue;
            }
            let mut j = end_prefix;
            let mut count = 0;
            while j < bytes.len() && count < *suffix_len {
                if is_alnum(bytes[j]) {
                    count += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if count == *suffix_len {
                // ensure boundary after memo if possible
                if j == bytes.len() || !is_alnum(bytes[j]) {
                    return Some(upper[i..j].to_string());
                }
            }
        }
    }
    None
}

fn parse_topup_created_at(created_at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
        .ok()
}

fn topup_payment_expired(created_at: &str, paid_at: DateTime<Utc>) -> bool {
    let Some(created) = parse_topup_created_at(created_at) else {
        return false;
    };
    paid_at.signed_duration_since(created) >= Duration::minutes(TOPUP_TTL_MINUTES)
}

fn extract_source_ip(headers: &HeaderMap) -> Option<String> {
    // Prefer Cloudflare header if present.
    for k in ["CF-Connecting-IP", "X-Real-IP", "X-Forwarded-For"] {
        if let Some(v) = headers.get(k).and_then(|v| v.to_str().ok()) {
            let s = v.split(',').next().unwrap_or("").trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn authorize_webhook(ctx: &AppContext, headers: &HeaderMap) -> bool {
    // Support both legacy header and SePay-style API key header.
    // - Legacy: X-Webhook-Secret: <secret>
    // - SePay:  Authorization: Apikey <secret>
    // - SePay UI key field: Authorization: <secret>

    // SePay-style
    if let Some(auth) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let auth = auth.trim();
        if auth == ctx.config.webhook_secret {
            return true;
        }

        // Accept: "Apikey <key>" with any casing for the scheme.
        if let Some((scheme, secret)) = auth.split_once(char::is_whitespace) {
            if scheme.eq_ignore_ascii_case("apikey") && secret.trim() == ctx.config.webhook_secret {
                return true;
            }
        }
    }

    // Legacy
    headers
        .get("X-Webhook-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|val| val == ctx.config.webhook_secret)
        .unwrap_or(false)
}

async fn handle_topup_webhook(
    ctx: &Arc<AppContext>,
    payload: &NormalizedPayment,
    provider: &str,
    source_ip: Option<&str>,
    raw_json: Option<&str>,
) -> Result<(StatusCode, Json<MessageResponse>), (StatusCode, String)> {
    let topup = wallet_repo::find_topup_by_memo(&ctx.pool, &payload.memo)
        .await
        .map_err(internal_error)?;

    let Some(topup) = topup else {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("rejected"),
            Some("topup memo not found"),
            raw_json,
        )
        .await;
        return Err((StatusCode::BAD_REQUEST, "topup memo not found".to_string()));
    };

    if topup.status == "completed" {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("ok"),
            Some("already completed"),
            raw_json,
        )
        .await;
        return Ok((
            StatusCode::OK,
            Json(MessageResponse {
                ok: true,
                message: "already completed".to_string(),
            }),
        ));
    }

    let completed_at =
        parse_paid_at(payload.paid_at.as_deref()).unwrap_or_else(|_| chrono::Utc::now());

    if topup.status == "expired" && topup_payment_expired(&topup.created_at, completed_at) {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("rejected"),
            Some("topup expired"),
            raw_json,
        )
        .await;
        return Err((StatusCode::BAD_REQUEST, "topup request expired".to_string()));
    }

    // Check expiry by the payment time, not by the webhook receive time.
    if topup_payment_expired(&topup.created_at, completed_at) {
        wallet_repo::expire_topup(&ctx.pool, topup.id).await.ok();
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("rejected"),
            Some("topup expired by payment time"),
            raw_json,
        )
        .await;
        return Err((StatusCode::BAD_REQUEST, "topup request expired".to_string()));
    }

    if payload.amount < topup.amount {
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("rejected"),
            Some("amount less than topup"),
            raw_json,
        )
        .await;
        return Err((
            StatusCode::BAD_REQUEST,
            "amount less than topup total".to_string(),
        ));
    }

    // Atomic + idempotent: complete topup + credit wallet.
    // If webhook retries/concurrent calls happen, only the first pending/expired -> completed update credits wallet.
    let mut tx = ctx.pool.begin().await.map_err(internal_error)?;
    let completed = wallet_repo::complete_paid_topup(&mut tx, topup.id)
        .await
        .map_err(internal_error)?;
    if !completed {
        tx.rollback().await.map_err(internal_error)?;
        let _ = repo::insert_webhook_event(
            &ctx.pool,
            provider,
            true,
            source_ip,
            Some(&payload.memo),
            Some(&payload.tx_id),
            Some(payload.amount),
            Some(&payload.status),
            None,
            Some("ok"),
            Some("already completed"),
            raw_json,
        )
        .await;
        return Ok((
            StatusCode::OK,
            Json(MessageResponse {
                ok: true,
                message: "already completed".to_string(),
            }),
        ));
    }

    let balance_after = wallet_repo::credit_wallet(
        &mut tx,
        topup.user_id,
        topup.amount,
        "topup",
        None,
        Some(topup.id),
        None,
    )
    .await
    .map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;

    let lang = i18n::user_lang_by_id(ctx, topup.user_id).await;
    let text = i18n::tr(
        ctx,
        &lang,
        "topup_success_message",
        "✅ Top-up successful!\n💰 Amount: {amount}\n🏦 New balance: {balance}\n🕒 Time: {completed_at}",
        &[
            (
                "amount",
                crate::bot::plugins::cmd_wallet::format_vnd(topup.amount),
            ),
            (
                "balance",
                crate::bot::plugins::cmd_wallet::format_vnd(balance_after),
            ),
            ("completed_at", format_vietnam_datetime(completed_at)),
        ],
    );
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            i18n::t(ctx, &lang, "start_btn_wallet", "💳 Wallet"),
            "start:wallet",
        ),
        InlineKeyboardButton::callback(
            i18n::t(ctx, &lang, "start_btn_shop", "🛒 Shop"),
            "start:shop",
        ),
    ]]);
    let _ = i18n::send_message_for_key(ctx, ChatId(topup.chat_id), "topup_success_message", text)
        .reply_markup(keyboard)
        .await;

    let _ = repo::insert_webhook_event(
        &ctx.pool,
        provider,
        true,
        source_ip,
        Some(&payload.memo),
        Some(&payload.tx_id),
        Some(payload.amount),
        Some(&payload.status),
        None,
        Some("ok"),
        None,
        raw_json,
    )
    .await;

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            ok: true,
            message: "topup processed".to_string(),
        }),
    ))
}

async fn credit_order_payment_to_wallet_response(
    ctx: &Arc<AppContext>,
    order: &OrderWithProduct,
    payload: &NormalizedPayment,
    provider: &str,
    source_ip: Option<&str>,
    raw_json: Option<&str>,
    status: OrderStatus,
    reason: &str,
) -> std::result::Result<(StatusCode, Json<MessageResponse>), (StatusCode, String)> {
    let balance_after =
        credit_paid_order_to_wallet(&ctx.pool, order, status, payload.amount, reason)
            .await
            .map_err(internal_error)?;

    let event_note = if balance_after.is_some() {
        "credited to wallet"
    } else {
        "already credited to wallet"
    };
    let _ = repo::insert_webhook_event(
        &ctx.pool,
        provider,
        true,
        source_ip,
        Some(&payload.memo),
        Some(&payload.tx_id),
        Some(payload.amount),
        Some(&payload.status),
        Some(&order.order.id),
        Some("ok"),
        Some(event_note),
        raw_json,
    )
    .await;

    if let Some(balance_after) = balance_after {
        if let Err(err) =
            notify_order_payment_credited_to_wallet(ctx, order, payload.amount, balance_after).await
        {
            warn!(
                "failed to notify user {} about wallet credit for order {}: {err}",
                order.order.user_id, order.order.id
            );
        }
    }

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            ok: true,
            message: event_note.to_string(),
        }),
    ))
}

async fn notify_order_payment_credited_to_wallet(
    ctx: &AppContext,
    order: &OrderWithProduct,
    amount: i64,
    balance_after: i64,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, order.order.user_id).await;
    let text = i18n::tr(
        ctx,
        &lang,
        "order_payment_credited_to_wallet",
        "✅ Payment received for order {memo}, but the order could not be delivered.\n💳 The amount {amount} has been added to your wallet.\n🏦 Wallet balance: {balance}",
        &[
            ("memo", order.order.bank_memo.clone()),
            ("amount", format_vnd(amount)),
            ("balance", format_vnd(balance_after)),
        ],
    );
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            i18n::t(ctx, &lang, "start_btn_wallet", "💳 Wallet"),
            "start:wallet",
        ),
        InlineKeyboardButton::callback(
            i18n::t(ctx, &lang, "start_btn_shop", "🛒 Shop"),
            "start:shop",
        ),
    ]]);
    i18n::send_message_for_key(
        ctx,
        ChatId(order.order.chat_id),
        "order_payment_credited_to_wallet",
        text,
    )
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

fn internal_error<E: std::fmt::Display>(err: E) -> (StatusCode, String) {
    warn!("Internal error: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error".to_string(),
    )
}

async fn credit_paid_order_to_wallet(
    pool: &SqlitePool,
    order: &OrderWithProduct,
    status: OrderStatus,
    amount: i64,
    reason: &str,
) -> Result<Option<i64>> {
    let mut tx = pool.begin().await?;
    if let Some(ids_str) = &order.order.reserved_item_ids {
        let ids = crate::domains::orders::api::parse_reserved_ids(ids_str);
        if !ids.is_empty() {
            repo::return_product_items(&mut tx, order.order.product_id, &ids).await?;
        }
    }

    let note =
        format!("Payment received but order was not delivered ({reason}); credited to wallet");
    let balance_after = wallet_repo::credit_order_payment_to_wallet_once(
        &mut tx,
        order.order.user_id,
        amount,
        &order.order.id,
        Some(&note),
    )
    .await?;
    repo::update_order_status_with_data(&mut tx, &order.order.id, status, None, None).await?;
    tx.commit().await?;
    Ok(balance_after)
}

use axum::Router;
use axum::routing::get;

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/webhook/payment", get(webhook_status).post(handle_webhook))
        .route(
            "/webhook/payment/",
            get(webhook_status).post(handle_webhook),
        )
}

async fn webhook_status() -> (StatusCode, Json<MessageResponse>) {
    (
        StatusCode::OK,
        Json(MessageResponse {
            ok: true,
            message: "webhook endpoint ready; send payment notifications with POST".to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::{
        orders::{models::Order, repo as orders_repo},
        wallet::repo as wallet_repo,
    };
    use crate::{app::AppContext, config::Config};
    use axum::{body::Body, http::Request};
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::{collections::HashMap, sync::Arc};
    use teloxide::Bot;
    use tower::ServiceExt;

    #[test]
    fn extracts_wallet_topup_memo_from_sepay_content() {
        assert_eq!(
            extract_memo_from_text("IBFT user nap tien NAPABC12345 cam on", "DH", 10),
            Some("NAPABC12345".to_string())
        );
    }

    #[test]
    fn extracts_order_memo_from_sepay_content() {
        assert_eq!(
            extract_memo_from_text("Thanh toan DHABC1234567", "DH", 10),
            Some("DHABC1234567".to_string())
        );
    }

    #[test]
    fn extracts_configured_order_memo_from_sepay_content() {
        assert_eq!(
            extract_memo_from_text("Thanh toan SHOPABC123DEF456 cam on", "SHOP", 12),
            Some("SHOPABC123DEF456".to_string())
        );
    }

    #[test]
    fn still_extracts_wallet_topup_memo_with_fixed_prefix() {
        assert_eq!(
            extract_memo_from_text("IBFT user nap tien NAPABC12345 cam on", "SHOP", 12),
            Some("NAPABC12345".to_string())
        );
    }

    #[test]
    fn does_not_extract_order_memo_below_configured_random_length() {
        assert_eq!(
            extract_memo_from_text("Thanh toan SHOPABC123DEF45", "SHOP", 12),
            None
        );
    }

    #[test]
    fn extracts_legacy_default_order_memo_from_sepay_content() {
        assert_eq!(
            extract_memo_from_text("Thanh toan DHABC12345", "DH", 8),
            Some("DHABC12345".to_string())
        );
    }

    #[test]
    fn normalizes_payment_memo_for_matching() {
        assert_eq!(normalize_payment_memo("  napabc12345  "), "NAPABC12345");
    }

    #[test]
    fn topup_expiry_uses_payment_time() {
        let created_at = "2026-07-21 10:00:00";
        let paid_in_time = DateTime::parse_from_rfc3339("2026-07-21T10:29:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let paid_late = DateTime::parse_from_rfc3339("2026-07-21T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(!topup_payment_expired(created_at, paid_in_time));
        assert!(topup_payment_expired(created_at, paid_late));
    }

    #[tokio::test]
    async fn webhook_endpoint_allows_get_health_check() {
        let response = router()
            .with_state(test_ctx())
            .oneshot(
                Request::get("/webhook/payment")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_endpoint_accepts_trailing_slash_health_check() {
        let response = router()
            .with_state(test_ctx())
            .oneshot(
                Request::get("/webhook/payment/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authorize_webhook_accepts_bare_authorization_secret() {
        let ctx = test_ctx();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "webhook-secret".parse().unwrap(),
        );

        assert!(authorize_webhook(&ctx, &headers));
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_ctx() -> Arc<AppContext> {
        let config = Config::from_env_map(&HashMap::from([
            ("TELOXIDE_TOKEN".to_string(), "test-token".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "test-admin-jwt-secret-at-least-32-chars".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup-code".to_string()),
            ("WEBHOOK_SECRET".to_string(), "webhook-secret".to_string()),
        ]))
        .unwrap();
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::new(),
            crate::bot::texts::BotTexts::default(),
            vec![],
        )
    }

    async fn seed_pending_order_with_reserved_item(pool: &SqlitePool) -> Order {
        sqlx::query("INSERT INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)")
            .bind(1_i64)
            .bind("Test product")
            .bind(50_000_i64)
            .bind(1_i64)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO product_items (id, product_id, content, is_buy) VALUES (?, ?, ?, ?)",
        )
        .bind(7_i64)
        .bind(1_i64)
        .bind("stock-item")
        .bind(1_i64)
        .execute(pool)
        .await
        .unwrap();

        let mut order = Order::new(
            42,
            420,
            1,
            1,
            50_000,
            "DHABC12345".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        order.reserved_item_ids = Some("7".to_string());
        orders_repo::insert_order(pool, &order).await.unwrap();
        order
    }

    #[tokio::test]
    async fn credit_paid_order_to_wallet_closes_order_and_returns_reserved_items() {
        let pool = test_pool().await;
        let order = seed_pending_order_with_reserved_item(&pool).await;
        let order_with_product = orders_repo::get_order_with_product(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();

        let credited = credit_paid_order_to_wallet(
            &pool,
            &order_with_product,
            OrderStatus::Expired,
            50_000,
            "order expired",
        )
        .await
        .unwrap();
        let credited_again = credit_paid_order_to_wallet(
            &pool,
            &order_with_product,
            OrderStatus::Expired,
            50_000,
            "order expired",
        )
        .await
        .unwrap();

        assert_eq!(credited, Some(50_000));
        assert_eq!(credited_again, None);
        let wallet = wallet_repo::get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 50_000);
        let returned: i64 = sqlx::query_scalar("SELECT is_buy FROM product_items WHERE id = 7")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(returned, 0);
        let updated = orders_repo::get_order_with_product(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.order.status, OrderStatus::Expired);
        assert_eq!(updated.order.reserved_item_ids, None);
    }
}
