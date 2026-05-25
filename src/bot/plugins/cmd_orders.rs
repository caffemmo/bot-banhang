use std::sync::Arc;
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, CallbackQuery, Message, User};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::domains::orders::models::{OrderStatus, OrderWithProduct};
use crate::domains::orders::repo;
use crate::domains::users::repo as users_repo;

pub struct OrdersCommandPlugin;

fn format_status(status: &OrderStatus) -> &'static str {
    match status {
        OrderStatus::Pending => "pending",
        OrderStatus::Paid => "paid",
        OrderStatus::Cancel => "cancel",
        OrderStatus::Expired => "expired",
    }
}

pub async fn send_orders(
    ctx: Arc<AppContext>,
    bot: teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    user: Option<&User>,
) -> anyhow::Result<()> {
    let Some(user) = user else {
        let text = ctx.get_text_lang("user_unknown", "en", "Cannot identify user.");
        i18n::send_message_for_key(&ctx, chat_id, "user_unknown", text).await?;
        return Ok(());
    };
    let preferred = users_repo::preferred_language(&ctx.pool, user.id.0 as i64)
        .await
        .ok()
        .flatten()
        .or_else(|| user.language_code.clone());
    let lang = ctx.normalize_language_code(preferred.as_deref());

    let orders = repo::list_paid_orders_for_user(&ctx.pool, user.id.0 as i64, 10).await?;
    if orders.is_empty() {
        let text = ctx.get_text_lang(
            "no_orders",
            &lang,
            "You do not have any successful orders yet.",
        );
        bot.send_message(chat_id, text)
            .reply_markup(orders_empty_keyboard())
            .await?;
        return Ok(());
    }

    let text = order_history_text(&lang);
    let keyboard = order_history_keyboard(&orders, &lang);
    bot.send_message(chat_id, text)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

fn order_history_text(_lang: &str) -> String {
    "🧾 LỊCH SỬ MUA HÀNG\n\nChọn mã đơn bên dưới để xem chi tiết.".to_string()
}

fn order_history_keyboard(orders: &[OrderWithProduct], _lang: &str) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for order in orders
        .iter()
        .filter(|order| matches!(order.order.status, OrderStatus::Paid))
    {
        rows.push(vec![InlineKeyboardButton::callback(
            order_button_label(order),
            format!("order:{}", order.order.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn product_is_active(order: &OrderWithProduct) -> bool {
    order.product.is_active.unwrap_or(1) != 0
}

fn order_detail_keyboard(order: &OrderWithProduct) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if product_is_active(order) {
        rows.push(vec![InlineKeyboardButton::callback(
            "🛒 Mua lại sản phẩm này",
            format!("buy:{}", order.product.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "💬 Hỗ trợ đơn này",
        format!("order_support:{}", order.order.id),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Lịch sử mua hàng",
        "orders:list",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🛒 Xem sản phẩm",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn order_not_found_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "⬅️ Lịch sử mua hàng",
            "orders:list",
        )],
        vec![InlineKeyboardButton::callback(
            "🛒 Xem sản phẩm",
            "start:shop",
        )],
    ])
}

fn orders_empty_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "🛒 Xem sản phẩm",
        "start:shop",
    )]])
}

fn order_button_label(order: &OrderWithProduct) -> String {
    let product_name = truncate_chars(order.product.name.trim(), 28);
    format!("{} — {}", order.order.id, product_name)
}

fn format_order_detail_text(order: &OrderWithProduct) -> String {
    let mut lines = vec![
        "🧾 CHI TIẾT ĐƠN HÀNG".to_string(),
        String::new(),
        format!("Mã đơn: {}", order.order.id),
        format!("Sản phẩm: {}", order.product.name),
    ];
    if !product_is_active(order) {
        lines.push("Trạng thái sản phẩm: Sản phẩm đã ngừng bán".to_string());
    }
    if let Some(plan_label) = order
        .order
        .plan_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Gói: {plan_label}"));
    }
    lines.extend([
        format!("Số lượng: {}", order.order.qty),
        format!("Tổng tiền: {}", format_vnd(order.order.amount)),
        format!("Trạng thái: {}", format_status(&order.order.status)),
        format!("Nội dung chuyển khoản: {}", order.order.bank_memo),
        format!("Thời gian tạo: {}", order.order.created_at),
        format!(
            "Thời gian thanh toán: {}",
            order.order.paid_at.as_deref().unwrap_or("-")
        ),
    ]);
    if let Some(tx_id) = order
        .order
        .payment_tx_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Mã giao dịch: {tx_id}"));
    }
    if let Some(customer_input) = order
        .order
        .customer_input
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thông tin đã nhập: {customer_input}"));
    }
    if let Some(delivered_data) = order
        .order
        .delivered_data
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push("Dữ liệu giao hàng:".to_string());
        lines.push(delivered_data.to_string());
    }
    lines.join("\n")
}

fn admin_support_message(order: &OrderWithProduct, user: &User) -> String {
    let username = user
        .username
        .as_deref()
        .map(|value| format!("@{value}"))
        .unwrap_or_else(|| "-".to_string());
    format!(
        "💬 Yêu cầu hỗ trợ đơn hàng\n\nOrder ID: {}\nUser ID: {}\nUsername: {}\nSản phẩm: {}\nNội dung CK: {}\nTổng tiền: {}",
        order.order.id,
        user.id.0,
        username,
        order.product.name,
        order.order.bank_memo,
        format_vnd(order.order.amount),
    )
}

async fn notify_admins_for_order_support(
    ctx: &Arc<AppContext>,
    order: &OrderWithProduct,
    user: &User,
) -> anyhow::Result<usize> {
    let admin_ids = ctx.telegram_icon_admin_ids();
    let mut sent = 0;
    for admin_id in admin_ids {
        if ctx
            .bot
            .send_message(
                teloxide::types::ChatId(admin_id),
                admin_support_message(order, user),
            )
            .await
            .is_ok()
        {
            sent += 1;
        }
    }
    Ok(sent)
}

fn format_vnd(amount: i64) -> String {
    let s = amount.abs().to_string();
    let mut with_sep = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            with_sep.push('.');
        }
        with_sep.push(ch);
    }
    let formatted: String = with_sep.chars().rev().collect();
    if amount < 0 {
        format!("-{}đ", formatted)
    } else {
        format!("{}đ", formatted)
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    format!("{}...", value.chars().take(max_chars).collect::<String>())
}

async fn show_orders_history(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    message_id: Option<teloxide::types::MessageId>,
    user_id: i64,
    lang: &str,
) -> anyhow::Result<()> {
    let orders = repo::list_paid_orders_for_user(&ctx.pool, user_id, 10).await?;
    if orders.is_empty() {
        let text = ctx.get_text_lang(
            "no_orders",
            lang,
            "You do not have any successful orders yet.",
        );
        if let Some(message_id) = message_id {
            ctx.bot
                .edit_message_text(chat_id, message_id, text)
                .reply_markup(orders_empty_keyboard())
                .await?;
        } else {
            ctx.bot
                .send_message(chat_id, text)
                .reply_markup(orders_empty_keyboard())
                .await?;
        }
        return Ok(());
    }
    let text = order_history_text(lang);
    let keyboard = order_history_keyboard(&orders, lang);
    if let Some(message_id) = message_id {
        ctx.bot
            .edit_message_text(chat_id, message_id, text)
            .reply_markup(keyboard)
            .await?;
    } else {
        ctx.bot
            .send_message(chat_id, text)
            .reply_markup(keyboard)
            .await?;
    }
    Ok(())
}

async fn handle_orders_callback(ctx: &Arc<AppContext>, q: CallbackQuery) -> anyhow::Result<()> {
    let data = q.data.clone().unwrap_or_default();
    let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
    let Some(ref msg) = q.message else {
        return Ok(());
    };
    let chat_id = msg.chat().id;
    let message_id = msg.id();
    let user_id = q.from.id.0 as i64;
    let lang = i18n::user_lang(ctx, user_id, q.from.language_code.as_deref()).await;

    if data == "orders:list" {
        show_orders_history(ctx, chat_id, Some(message_id), user_id, &lang).await?;
        return Ok(());
    }

    if let Some(order_id) = data.strip_prefix("order_support:") {
        let Some(order) = repo::get_paid_order_for_user(&ctx.pool, order_id, user_id).await? else {
            ctx.bot
                .edit_message_text(
                    chat_id,
                    message_id,
                    ctx.get_text_lang("order_not_found", &lang, "Order not found."),
                )
                .reply_markup(order_not_found_keyboard())
                .await?;
            return Ok(());
        };
        let sent = notify_admins_for_order_support(ctx, &order, &q.from).await?;
        let text = if sent > 0 {
            "✅ Đã gửi yêu cầu hỗ trợ cho admin. Vui lòng chờ phản hồi."
        } else {
            "⚠️ Chưa cấu hình Telegram admin ID để nhận hỗ trợ."
        };
        ctx.bot
            .send_message(chat_id, text)
            .reply_markup(order_not_found_keyboard())
            .await?;
        return Ok(());
    }

    let Some(order_id) = data.strip_prefix("order:") else {
        return Ok(());
    };
    let Some(order) = repo::get_paid_order_for_user(&ctx.pool, order_id, user_id).await? else {
        ctx.bot
            .edit_message_text(
                chat_id,
                message_id,
                ctx.get_text_lang("order_not_found", &lang, "Order not found."),
            )
            .reply_markup(order_not_found_keyboard())
            .await?;
        return Ok(());
    };

    ctx.bot
        .edit_message_text(chat_id, message_id, format_order_detail_text(&order))
        .reply_markup(order_detail_keyboard(&order))
        .await?;
    Ok(())
}

#[async_trait::async_trait]
impl AppPlugin for OrdersCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdOrders"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![
            BotCommand {
                command: "orders".to_string(),
                description: "Recent orders".to_string(),
            },
            BotCommand {
                command: "order".to_string(),
                description: "Find order by ID".to_string(),
            },
        ]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("");

        if text.starts_with("/orders") {
            send_orders(ctx.clone(), ctx.bot.clone(), msg.chat.id, msg.from()).await?;
            let _ = dialogue.update(State::Idle).await;
            return Ok(true);
        }

        if text.starts_with("/order") {
            let order_id = text
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            let Some(user) = msg.from() else {
                let text = ctx.get_text_lang("user_unknown", "en", "Cannot identify user.");
                i18n::send_message_for_key(&ctx, msg.chat.id, "user_unknown", text).await?;
                return Ok(true);
            };
            let lang = i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await;
            if order_id.is_empty() {
                ctx.bot
                    .send_message(msg.chat.id, "Dùng: /order <order_id>")
                    .reply_markup(orders_empty_keyboard())
                    .await?;
                return Ok(true);
            }
            if let Some(order) =
                repo::get_paid_order_for_user(&ctx.pool, &order_id, user.id.0 as i64).await?
            {
                ctx.bot
                    .send_message(msg.chat.id, format_order_detail_text(&order))
                    .reply_markup(order_detail_keyboard(&order))
                    .await?;
            } else {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        ctx.get_text_lang("order_not_found", &lang, "Order not found."),
                    )
                    .reply_markup(order_not_found_keyboard())
                    .await?;
            }
            let _ = dialogue.update(State::Idle).await;
            return Ok(true);
        }

        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let data = q.data.clone().unwrap_or_default();
        if data == "orders:list" || data.starts_with("order:") || data.starts_with("order_support:")
        {
            handle_orders_callback(&ctx, q).await?;
            return Ok(true);
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::orders::models::{Order, OrderReservationMode, OrderWithProduct};
    use crate::domains::products::models::Product;

    fn test_order(id: &str, status: OrderStatus, memo: &str) -> OrderWithProduct {
        OrderWithProduct {
            order: Order {
                id: id.to_string(),
                user_id: 100,
                chat_id: 200,
                product_id: 1,
                qty: 2,
                amount: 120_000,
                status,
                bank_memo: memo.to_string(),
                created_at: "2026-05-24T10:00:00Z".to_string(),
                paid_at: Some("2026-05-24T10:02:00Z".to_string()),
                payment_tx_id: Some("TX123".to_string()),
                delivered_data: Some("license-key-1".to_string()),
                reserved_item_ids: None,
                customer_input: Some("buyer@example.com".to_string()),
                plan_id: None,
                plan_label: Some("1 tháng".to_string()),
                plan_months: Some(1),
                plan_price: Some(120_000),
                reservation_mode: OrderReservationMode::Reserved,
            },
            product: Product {
                id: 1,
                name: "Gói VIP".to_string(),
                price: 60_000,
                is_active: Some(1),
                requires_input: Some(0),
                input_prompt: None,
                description: None,
                image_url: None,
                delivery_type: Some("stock_item".to_string()),
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

    #[test]
    fn order_history_keyboard_uses_order_id_callbacks_for_paid_orders_only() {
        let paid = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let pending = test_order("order-pending-1", OrderStatus::Pending, "DHPEND1234");

        let keyboard = order_history_keyboard(&[paid, pending], "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["callback_data"], "order:order-paid-1");
        assert!(rows[0][0]["text"].as_str().unwrap().contains("Gói VIP"));
        assert!(
            rows[0][0]["text"]
                .as_str()
                .unwrap()
                .contains("order-paid-1")
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0]["callback_data"], "start:shop");
    }

    #[test]
    fn order_history_keyboard_uses_back_label_for_shop_return() {
        let paid = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");

        let keyboard = order_history_keyboard(&[paid], "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["text"], "⬅️ Quay lại");
        assert_eq!(last_row[0]["callback_data"], "start:shop");
    }

    #[test]
    fn orders_empty_keyboard_has_back_to_shop() {
        let keyboard = orders_empty_keyboard();
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:shop"));
    }

    #[test]
    fn order_detail_text_contains_full_paid_order_information() {
        let order = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");

        let text = format_order_detail_text(&order);

        assert!(text.contains("order-paid-1"));
        assert!(text.contains("Gói VIP"));
        assert!(text.contains("2"));
        assert!(text.contains("120.000đ"));
        assert!(text.contains("paid"));
        assert!(text.contains("DHPAID1234"));
        assert!(text.contains("2026-05-24T10:00:00Z"));
        assert!(text.contains("2026-05-24T10:02:00Z"));
        assert!(text.contains("TX123"));
        assert!(text.contains("license-key-1"));
        assert!(text.contains("buyer@example.com"));
    }

    #[test]
    fn order_detail_keyboard_allows_rebuy_only_for_active_products() {
        let active = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let mut inactive = active.clone();
        inactive.product.is_active = Some(0);

        let active_keyboard = order_detail_keyboard(&active);
        let active_json = serde_json::to_value(&active_keyboard).unwrap();
        let active_rows = active_json["inline_keyboard"].as_array().unwrap();
        assert_eq!(active_rows[0][0]["callback_data"], "buy:1");

        let inactive_keyboard = order_detail_keyboard(&inactive);
        let inactive_json = serde_json::to_value(&inactive_keyboard).unwrap();
        let inactive_rows = inactive_json["inline_keyboard"].as_array().unwrap();
        let callbacks = inactive_rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();
        assert!(!callbacks.contains(&"buy:1"));
    }

    #[test]
    fn order_detail_text_marks_inactive_product_as_unavailable() {
        let mut order = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        order.product.is_active = Some(0);

        let text = format_order_detail_text(&order);

        assert!(text.contains("Sản phẩm đã ngừng bán"));
    }
}
