use anyhow::Result;
use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};
use tracing::warn;

use crate::app::AppContext;
use crate::bot::i18n;
use crate::core::time::format_vietnam_datetime;
use crate::domains::orders::fulfillment::PaymentSource;
use crate::domains::orders::models::OrderWithProduct;
use crate::domains::users::repo as users_repo;

const ADMIN_ORDER_PAID_NOTIFICATION_KEY: &str = "admin_order_paid_notification";

pub async fn notify_admins_order_paid(
    ctx: &AppContext,
    order: &OrderWithProduct,
    payment_ref: &str,
    paid_at: DateTime<Utc>,
    source: &PaymentSource,
) -> Result<()> {
    if !ctx.order_notifications_enabled() {
        return Ok(());
    }

    let admin_ids = ctx.order_notification_admin_ids();
    if admin_ids.is_empty() {
        return Ok(());
    }

    let username = order_user_display(ctx, order.order.user_id).await;

    for admin_id in admin_ids {
        let lang = i18n::user_lang_by_id(ctx, admin_id).await;
        let text = render_admin_order_paid_notification(
            ctx,
            &lang,
            order,
            payment_ref,
            paid_at,
            payment_source_label(source),
            &username,
        );
        if let Err(err) = i18n::send_message_for_key(
            ctx,
            ChatId(admin_id),
            ADMIN_ORDER_PAID_NOTIFICATION_KEY,
            text,
        )
        .reply_markup(admin_refund_request_keyboard(&order.order.id))
        .await
        {
            warn!("send paid-order admin notification failed for {admin_id}: {err}");
        }
    }

    Ok(())
}

pub fn render_admin_order_paid_notification(
    _ctx: &AppContext,
    _lang: &str,
    order: &OrderWithProduct,
    _payment_ref: &str,
    paid_at: DateTime<Utc>,
    _source_label: &str,
    _username: &str,
) -> String {
    format!(
        "✅ CÓ ĐƠN THANH TOÁN THÀNH CÔNG\n🔖 Nội dung CK: {}\nSản phẩm: {}\nThời gian: {}",
        order.order.bank_memo,
        order.product.name,
        format_vietnam_datetime(paid_at)
    )
}

pub fn payment_source_label(source: &PaymentSource) -> &'static str {
    match source {
        PaymentSource::BankWebhook { .. } => "bank_webhook",
        PaymentSource::BinancePay { .. } => "binance_pay",
        PaymentSource::Bep20 { .. } => "bep20",
        PaymentSource::AdminManual { .. } => "admin_manual",
        PaymentSource::Wallet => "wallet",
        PaymentSource::ClientApiWallet => "client_api_wallet",
    }
}

pub fn admin_refund_request_keyboard(order_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "💸 Hoàn tiền",
            format!("admin_refund:req:{order_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "🧾 Xem thông tin đơn hàng",
            format!("admin_order:view:{order_id}"),
        )],
    ])
}

pub fn admin_refund_confirm_keyboard(order_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "✅ Xác nhận hoàn tiền",
            format!("admin_refund:confirm:{order_id}"),
        ),
        InlineKeyboardButton::callback("❌ Huỷ lệnh", format!("admin_refund:cancel:{order_id}")),
    ]])
}

pub async fn order_user_display(ctx: &AppContext, user_id: i64) -> String {
    users_repo::get_subscriber_by_user_id(&ctx.pool, user_id)
        .await
        .ok()
        .flatten()
        .and_then(|subscriber| subscriber.username)
        .map(|username| username.trim().trim_start_matches('@').to_string())
        .filter(|username| !username.is_empty())
        .map(|username| format!("@{username}"))
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;
    use teloxide::Bot;

    use super::*;
    use crate::bot::texts::{BotTexts, LanguageInfo};
    use crate::config::{Config, CryptoConfig};
    use crate::domains::orders::models::{Order, OrderReservationMode, OrderStatus};
    use crate::domains::products::models::Product;

    #[tokio::test]
    async fn renders_compact_admin_paid_order_notification() {
        let ctx = test_ctx(BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "vi".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([(
                    "admin_order_paid_notification".to_string(),
                    "Order: {order_id}; plan {plan}; amount {amount}; memo {memo}"
                        .to_string(),
                )]),
            )]),
        ));
        let order = order_with_product();
        let paid_at = Utc.with_ymd_and_hms(2026, 5, 26, 1, 2, 3).unwrap();

        let text = render_admin_order_paid_notification(
            &ctx,
            "vi",
            &order,
            "tx-123",
            paid_at,
            "bank_webhook",
            "@alice",
        );

        assert_eq!(
            text,
            "✅ CÓ ĐƠN THANH TOÁN THÀNH CÔNG\n🔖 Nội dung CK: MEMO1\nSản phẩm: Test product\nThời gian: 26/05/2026 08:02:03"
        );
    }

    #[test]
    fn refund_request_keyboard_targets_order() {
        let keyboard = admin_refund_request_keyboard("order-123");
        let json = serde_json::to_value(&keyboard).unwrap();

        assert_eq!(
            json["inline_keyboard"][0][0]["callback_data"],
            "admin_refund:req:order-123"
        );
        assert_eq!(
            json["inline_keyboard"][1][0]["callback_data"],
            "admin_order:view:order-123"
        );
    }

    #[test]
    fn payment_source_labels_include_wallet_flows() {
        assert_eq!(payment_source_label(&PaymentSource::Wallet), "wallet");
        assert_eq!(
            payment_source_label(&PaymentSource::ClientApiWallet),
            "client_api_wallet"
        );
    }

    fn test_ctx(texts: BotTexts) -> std::sync::Arc<AppContext> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: CryptoConfig::default(),
            },
            HashMap::new(),
            texts,
            vec![],
        )
    }

    fn order_with_product() -> OrderWithProduct {
        let mut order = Order::new(
            42,
            420,
            1,
            2,
            50_000,
            "MEMO1".to_string(),
            Some("customer@example.test".to_string()),
            None,
            None,
            None,
            None,
        );
        order.status = OrderStatus::Paid;
        order.reservation_mode = OrderReservationMode::Reserved;

        OrderWithProduct {
            order,
            product: Product {
                id: 1,
                name: "Test product".to_string(),
                price: 25_000,
                is_active: Some(1),
                requires_input: None,
                input_prompt: None,
                description: None,
                image_url: None,
                delivery_type: None,
                file_path: None,
                file_name: None,
                file_mime: None,
                category_id: None,
                category: None,
                category_emoji: None,
                category_custom_emoji_id: None,
                button_emoji: None,
                button_custom_emoji_id: None,
                created_at: None,
                sort_order: None,
                show_sold_count: Some(0),
            },
        }
    }
}
