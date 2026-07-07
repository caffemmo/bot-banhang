use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use teloxide::{
    payloads::SendMessageSetters,
    types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup},
};
use tokio::time;
use tracing::{error, info};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::domains::crypto_pay::binance_worker;
use crate::domains::crypto_pay::repo as crypto_repo;
use crate::domains::crypto_pay::worker as crypto_worker;
use crate::domains::orders::api::{RESERVE_TTL_MINUTES, release_reservation};
use crate::domains::orders::models::OrderStatus;
use crate::domains::orders::webhook::TOPUP_TTL_MINUTES;
use crate::domains::products::repo;
use crate::domains::wallet::repo as wallet_repo;

const ORDER_RETENTION_DAYS: i64 = 7;

pub async fn run(ctx: Arc<AppContext>) -> Result<()> {
    let mut cleanup_ticker = time::interval(time::Duration::from_secs(60));

    loop {
        cleanup_ticker.tick().await;
        run_cleanup_tick(&ctx).await;
    }
}

async fn run_cleanup_tick(ctx: &Arc<AppContext>) {
    let cutoff = order_cleanup_cutoff(Utc::now());
    match repo::list_pending_before(&ctx.pool, &cutoff).await {
        Ok(pending) => {
            for order in pending {
                if let Err(err) = release_reservation(ctx, &order, OrderStatus::Expired).await {
                    error!(
                        "release_reservation failed for order {}: {err}",
                        order.order.id
                    );
                    continue;
                }

                let lang = i18n::user_lang_by_id(ctx, order.order.user_id).await;
                let text = i18n::tr(
                    ctx,
                    &lang,
                    "order_expired_message",
                    "⌛ Order {memo} expired after {ttl} minutes waiting for payment.\n🛒 You can create a new order in /shop.",
                    &[
                        ("memo", order.order.bank_memo.clone()),
                        ("ttl", RESERVE_TTL_MINUTES.to_string()),
                    ],
                );
                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        i18n::t(ctx, &lang, "open_shop_btn", "🛒 Open shop"),
                        "start:shop",
                    )]]);

                if let Err(err) = i18n::send_message_for_key(
                    ctx,
                    ChatId(order.order.chat_id),
                    "order_expired_message",
                    text,
                )
                .reply_markup(keyboard)
                .await
                {
                    error!(
                        "failed to notify chat {} about expired order {}: {err}",
                        order.order.chat_id, order.order.id
                    );
                }
            }
        }
        Err(err) => error!("list_pending_before failed: {err}"),
    }

    let topup_cutoff = (Utc::now() - Duration::minutes(TOPUP_TTL_MINUTES))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    match wallet_repo::list_pending_topups_before(&ctx.pool, &topup_cutoff).await {
        Ok(stale_topups) => {
            for topup in stale_topups {
                if let Err(e) = wallet_repo::expire_topup(&ctx.pool, topup.id).await {
                    error!("expire_topup failed for topup {}: {e}", topup.id);
                    continue;
                }
                let lang = i18n::user_lang_by_id(ctx, topup.user_id).await;
                let text = i18n::tr(
                    ctx,
                    &lang,
                    "topup_expired_message",
                    "⌛ Top-up request {amount} (memo: {memo}) expired after {ttl} minutes.",
                    &[
                        (
                            "amount",
                            crate::bot::plugins::cmd_wallet::format_vnd(topup.amount),
                        ),
                        ("memo", topup.memo.clone()),
                        ("ttl", TOPUP_TTL_MINUTES.to_string()),
                    ],
                );
                if let Err(e) = i18n::send_message_for_key(
                    ctx,
                    ChatId(topup.chat_id),
                    "topup_expired_message",
                    text,
                )
                .await
                {
                    error!(
                        "failed to notify chat {} about expired topup {}: {e}",
                        topup.chat_id, topup.id
                    );
                }
            }
        }
        Err(e) => error!("list_pending_topups_before failed: {e}"),
    }

    let crypto_cutoff = crypto_cleanup_cutoff(Utc::now());
    match crypto_repo::list_pending_crypto_payments_before(&ctx.pool, &crypto_cutoff).await {
        Ok(stale_payments) => {
            for payment in stale_payments {
                match crypto_repo::expire_crypto_payment(&ctx.pool, payment.id).await {
                    Ok(true) => {
                        let lang = i18n::user_lang_by_id(ctx, payment.user_id).await;
                        let text = i18n::tr(
                            ctx,
                            &lang,
                            "crypto_payment_expired_message",
                            "⌛ USDT payment request for order {order_id} has expired. Create a new payment request if you still want to pay.",
                            &[(
                                "order_id",
                                payment
                                    .order_id
                                    .clone()
                                    .unwrap_or_else(|| format!("USDT-{}", payment.id)),
                            )],
                        );
                        if let Err(err) = i18n::send_message_for_key(
                            ctx,
                            ChatId(payment.chat_id),
                            "crypto_payment_expired_message",
                            text,
                        )
                        .await
                        {
                            error!(
                                "failed to notify chat {} about expired crypto payment {}: {err}",
                                payment.chat_id, payment.id
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(err) => error!("expire_crypto_payment failed for {}: {err}", payment.id),
                }
            }
        }
        Err(err) => error!("list_pending_crypto_payments_before failed: {err}"),
    }

    if ctx.bep20_enabled()
        && let Err(err) = crypto_worker::run_bep20_tick(ctx.clone()).await
    {
        error!("BEP20 worker tick failed: {err}");
    }

    if ctx.binance_pay_enabled()
        && let Err(err) = binance_worker::run_binance_pay_tick(ctx.clone()).await
    {
        error!("Binance Pay note worker tick failed: {err}");
    }

    match delete_orders_older_than(&ctx.pool, Utc::now()).await {
        Ok(deleted) if deleted > 0 => {
            info!("deleted {deleted} orders older than {ORDER_RETENTION_DAYS} days");
        }
        Ok(_) => {}
        Err(err) => error!("delete old orders failed: {err}"),
    }

    info!("pending cleanup tick finished");
}

fn order_cleanup_cutoff(now: DateTime<Utc>) -> String {
    (now - Duration::minutes(RESERVE_TTL_MINUTES)).to_rfc3339()
}

fn order_retention_cutoff(now: DateTime<Utc>) -> String {
    (now - Duration::days(ORDER_RETENTION_DAYS)).to_rfc3339()
}

async fn delete_orders_older_than(pool: &sqlx::SqlitePool, now: DateTime<Utc>) -> Result<u64> {
    let cutoff = order_retention_cutoff(now);
    let result = sqlx::query("DELETE FROM orders WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

fn crypto_cleanup_cutoff(now: DateTime<Utc>) -> String {
    now.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn order_cleanup_cutoff_uses_same_rfc3339_format_as_orders() {
        let now = Utc.with_ymd_and_hms(2026, 5, 14, 10, 6, 0).unwrap();

        let cutoff = order_cleanup_cutoff(now);

        assert_eq!(cutoff, "2026-05-14T10:01:00+00:00");
    }

    #[test]
    fn order_retention_cutoff_keeps_recent_week_of_orders() {
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 10, 6, 0).unwrap();

        let cutoff = order_retention_cutoff(now);

        assert_eq!(cutoff, "2026-06-08T10:06:00+00:00");
    }

    #[test]
    fn crypto_cleanup_cutoff_uses_current_rfc3339_time() {
        let now = Utc.with_ymd_and_hms(2026, 5, 21, 10, 6, 0).unwrap();

        let cutoff = crypto_cleanup_cutoff(now);

        assert_eq!(cutoff, "2026-05-21T10:06:00+00:00");
    }
}
