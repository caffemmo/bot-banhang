use anyhow::Result;
use chrono::{DateTime, Utc};
use teloxide::types::ChatId;
use tracing::warn;

use crate::app::AppContext;
use crate::bot::i18n;
use crate::domains::orders::fulfillment::PaymentSource;
use crate::domains::orders::models::OrderWithProduct;

const ADMIN_ORDER_PAID_NOTIFICATION_KEY: &str = "admin_order_paid_notification";
const ADMIN_ORDER_PAID_NOTIFICATION_DEFAULT: &str = "✅ New paid order\n\nOrder: {order_id}\nMemo: {memo}\nProduct: {product}\nPlan: {plan}\nQuantity: {qty}\nAmount: {amount} VND\nCustomer: {customer}\nUser ID: {user_id}\nChat ID: {chat_id}\nPayment ref: {payment_ref}\nSource: {source}\nPaid at: {paid_at}";

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

    for admin_id in admin_ids {
        let lang = i18n::user_lang_by_id(ctx, admin_id).await;
        let text = render_admin_order_paid_notification(
            ctx,
            &lang,
            order,
            payment_ref,
            paid_at,
            payment_source_label(source),
        );
        if let Err(err) = i18n::send_message_for_key(
            ctx,
            ChatId(admin_id),
            ADMIN_ORDER_PAID_NOTIFICATION_KEY,
            text,
        )
        .await
        {
            warn!("send paid-order admin notification failed for {admin_id}: {err}");
        }
    }

    Ok(())
}

pub fn render_admin_order_paid_notification(
    ctx: &AppContext,
    lang: &str,
    order: &OrderWithProduct,
    payment_ref: &str,
    paid_at: DateTime<Utc>,
    source_label: &str,
) -> String {
    let plan = order
        .order
        .plan_label
        .clone()
        .unwrap_or_else(|| i18n::t(ctx, lang, "delivery_plan_none", "None"));
    let customer = order
        .order
        .customer_input
        .clone()
        .unwrap_or_else(|| i18n::t(ctx, lang, "delivery_customer_none", "Not provided"));

    i18n::tr(
        ctx,
        lang,
        ADMIN_ORDER_PAID_NOTIFICATION_KEY,
        ADMIN_ORDER_PAID_NOTIFICATION_DEFAULT,
        &[
            ("order_id", order.order.id.clone()),
            ("memo", order.order.bank_memo.clone()),
            ("product", order.product.name.clone()),
            ("plan", plan),
            ("qty", order.order.qty.to_string()),
            ("amount", order.order.amount.to_string()),
            ("customer", customer),
            ("user_id", order.order.user_id.to_string()),
            ("chat_id", order.order.chat_id.to_string()),
            ("payment_ref", payment_ref.to_string()),
            ("source", source_label.to_string()),
            ("paid_at", paid_at.to_rfc3339()),
        ],
    )
}

pub fn payment_source_label(source: &PaymentSource) -> &'static str {
    match source {
        PaymentSource::BankWebhook { .. } => "bank_webhook",
        PaymentSource::BinancePay { .. } => "binance_pay",
        PaymentSource::Bep20 { .. } => "bep20",
        PaymentSource::AdminManual { .. } => "admin_manual",
    }
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
    async fn renders_configurable_admin_paid_order_notification_with_order_vars() {
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
                    "Đơn {memo}: {product} x{qty} = {amount}; ref {payment_ref}; {source}; {paid_at}"
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
        );

        assert_eq!(
            text,
            "Đơn MEMO1: Test product x2 = 50000; ref tx-123; bank_webhook; 2026-05-26T01:02:03+00:00"
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
            },
        }
    }
}
