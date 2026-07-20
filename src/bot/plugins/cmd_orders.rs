use std::sync::Arc;
use teloxide::payloads::{
    AnswerCallbackQuerySetters, EditMessageTextSetters, SendDocumentSetters, SendMessageSetters,
};
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, CallbackQuery, InputFile, Message, ParseMode, User};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::core::time::format_optional_vietnam_time;
use crate::domains::orders::admin_notify::{
    admin_refund_confirm_keyboard, admin_refund_request_keyboard, order_user_display,
};
use crate::domains::orders::models::{OrderStatus, OrderWithProduct};
use crate::domains::orders::api::{
    cookie_message_html, format_cookie_text, parse_account_delivery_items,
};
use crate::domains::orders::refund::refund_paid_order_to_wallet;
use crate::domains::orders::repo;
use crate::domains::users::repo as users_repo;

pub struct OrdersCommandPlugin;

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
            .reply_markup(orders_empty_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    let text = order_history_text(&ctx, &lang);
    let keyboard = order_history_keyboard_json(&ctx, &lang, &orders);
    i18n::send_message_with_json_keyboard(&ctx, chat_id, "orders_history_text", text, keyboard)
        .await?;
    Ok(())
}

fn order_history_text(ctx: &AppContext, lang: &str) -> String {
    i18n::t(
        ctx,
        lang,
        "orders_history_text",
        "🧾 LỊCH SỬ MUA HÀNG\n\nChọn nội dung chuyển khoản bên dưới để xem chi tiết.",
    )
}

fn order_history_keyboard(
    ctx: &AppContext,
    lang: &str,
    orders: &[OrderWithProduct],
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for order in orders
        .iter()
        .filter(|order| matches!(order.order.status, OrderStatus::Paid))
    {
        rows.push(vec![InlineKeyboardButton::callback(
            order_button_label(ctx, lang, order),
            format!("order:{}", order.order.id),
        )]);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "orders_back_shop_btn",
        "⬅️ Quay lại",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn order_history_keyboard_json(
    ctx: &AppContext,
    lang: &str,
    orders: &[OrderWithProduct],
) -> serde_json::Value {
    let mut rows = Vec::new();
    for order in orders
        .iter()
        .filter(|order| matches!(order.order.status, OrderStatus::Paid))
    {
        let parts = i18n::button_parts_for_key(
            ctx,
            "orders_history_button",
            order_button_template_text(ctx, lang, order),
        );
        let mut button = serde_json::json!({
            "text": parts.text,
            "callback_data": format!("order:{}", order.order.id),
        });
        if let Some(icon_id) = parts.icon_custom_emoji_id
            && let Some(obj) = button.as_object_mut()
        {
            obj.insert("icon_custom_emoji_id".to_string(), serde_json::Value::String(icon_id));
        }
        rows.push(vec![button]);
    }
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "orders_back_shop_btn",
        "⬅️ Quay lại",
        "start:shop",
    )]);
    serde_json::json!({ "inline_keyboard": rows })
}

fn product_is_active(order: &OrderWithProduct) -> bool {
    order.product.is_active.unwrap_or(1) != 0
}

fn order_detail_keyboard(
    ctx: &AppContext,
    lang: &str,
    order: &OrderWithProduct,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if product_is_active(order) {
        rows.push(vec![i18n::inline_button_callback(
            ctx,
            lang,
            "order_rebuy_btn",
            "🛒 Mua lại sản phẩm này",
            format!("buy:{}", order.product.id),
        )]);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "order_support_btn",
        "💬 Hỗ trợ đơn này",
        format!("order_support:{}", order.order.id),
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "orders_history_back_btn",
        "⬅️ Lịch sử mua hàng",
        "orders:list",
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "orders_empty_shop_btn",
        "🛒 Xem sản phẩm",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn order_not_found_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "orders_history_back_btn",
            "⬅️ Lịch sử mua hàng",
            "orders:list",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "orders_empty_shop_btn",
            "🛒 Xem sản phẩm",
            "start:shop",
        )],
    ])
}

fn orders_empty_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![i18n::inline_button_callback(
        ctx,
        lang,
        "orders_empty_shop_btn",
        "🛒 Xem sản phẩm",
        "start:shop",
    )]])
}

fn order_button_label(ctx: &AppContext, lang: &str, order: &OrderWithProduct) -> String {
    i18n::button_parts_for_key(
        ctx,
        "orders_history_button",
        order_button_template_text(ctx, lang, order),
    )
    .text
}

fn order_button_template_text(ctx: &AppContext, lang: &str, order: &OrderWithProduct) -> String {
    let product_name = truncate_chars(&clean_order_product_name(&order.product.name), 26);
    let datetime = format_optional_vietnam_time(order.order.paid_at.as_deref());
    let date = order_paid_date(&datetime);
    i18n::tr(
        ctx,
        lang,
        "orders_history_button",
        "{memo} • {date} • {product}",
        &[
            ("memo", order.order.bank_memo.clone()),
            ("date", date),
            ("datetime", datetime),
            ("product", product_name),
            ("amount", format_vnd(order.order.amount)),
            ("qty", order.order.qty.to_string()),
        ],
    )
}

fn order_paid_date(datetime: &str) -> String {
    if datetime.len() >= 5 && datetime.as_bytes().get(2) == Some(&b'/') {
        datetime.chars().take(5).collect()
    } else {
        "-".to_string()
    }
}

fn clean_order_product_name(value: &str) -> String {
    let mut cleaned = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        cleaned.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        if let Some(end) = after_start.find('}') {
            let candidate = &after_start[..end];
            if !candidate.is_empty() && candidate.chars().all(|ch| ch.is_ascii_digit()) {
                rest = &after_start[end + 1..];
                continue;
            }
        }
        cleaned.push('{');
        rest = after_start;
    }
    cleaned.push_str(rest);
    cleaned
        .trim()
        .trim_start_matches(['-', '—', '|', '•', ':', ' '])
        .trim()
        .to_string()
}

fn order_detail_line(
    ctx: &AppContext,
    lang: &str,
    key: &str,
    default_label: &str,
    value: impl std::fmt::Display,
) -> String {
    format!("{}: {}", i18n::t(ctx, lang, key, default_label), value)
}

fn format_status(ctx: &AppContext, lang: &str, status: &OrderStatus) -> String {
    match status {
        OrderStatus::Pending => i18n::t(ctx, lang, "order_status_pending", "pending"),
        OrderStatus::Paid => i18n::t(ctx, lang, "order_status_paid", "paid"),
        OrderStatus::Refunded => i18n::t(ctx, lang, "order_status_refunded", "refunded"),
        OrderStatus::Cancel => i18n::t(ctx, lang, "order_status_cancel", "cancel"),
        OrderStatus::Expired => i18n::t(ctx, lang, "order_status_expired", "expired"),
    }
}

async fn handle_admin_refund_callback(
    ctx: &Arc<AppContext>,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let data = q.data.clone().unwrap_or_default();
    let admin_id = q.from.id.0 as i64;
    if !ctx
        .order_notification_admin_ids()
        .into_iter()
        .any(|allowed| allowed == admin_id)
    {
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Bạn không có quyền hoàn tiền.")
            .show_alert(true)
            .await;
        return Ok(());
    }

    let Some(ref msg) = q.message else {
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        return Ok(());
    };
    let chat_id = msg.chat().id;
    let message_id = msg.id();
    let lang = i18n::user_lang(ctx, admin_id, q.from.language_code.as_deref()).await;

    if let Some(order_id) = data.strip_prefix("admin_order:view:") {
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(order) = repo::get_order_with_product(&ctx.pool, order_id).await? else {
            ctx.bot
                .send_message(chat_id, "Không tìm thấy đơn hàng.")
                .await?;
            return Ok(());
        };
        ctx.bot
            .send_message(chat_id, format_order_detail_text(ctx, &lang, &order))
            .reply_markup(admin_refund_request_keyboard(&order.order.id))
            .await?;
        return Ok(());
    }

    if let Some(order_id) = data.strip_prefix("admin_refund:req:") {
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(order) = repo::get_order_with_product(&ctx.pool, order_id).await? else {
            ctx.bot
                .send_message(chat_id, "Không tìm thấy đơn để hoàn tiền.")
                .await?;
            return Ok(());
        };
        if !matches!(
            order.order.status,
            OrderStatus::Paid | OrderStatus::Refunded
        ) {
            ctx.bot
                .send_message(
                    chat_id,
                    format!(
                        "Không thể hoàn tiền đơn {} vì trạng thái hiện tại là {}.",
                        order.order.id,
                        order.order.status.to_string()
                    ),
                )
                .await?;
            return Ok(());
        }
        let username = order_user_display(ctx, order.order.user_id).await;
        let text = format!(
            "Xác nhận hoàn tiền\n\nOrder: {}\nUser ID: {}\nUsername: {}\nSố tiền: {}\n\nTiền sẽ được cộng vào ví user.",
            order.order.id,
            order.order.user_id,
            username,
            format_vnd(order.order.amount),
        );
        ctx.bot
            .send_message(chat_id, text)
            .reply_markup(admin_refund_confirm_keyboard(&order.order.id))
            .await?;
        return Ok(());
    }

    if let Some(order_id) = data.strip_prefix("admin_refund:cancel:") {
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Đã huỷ lệnh hoàn tiền.")
            .await;
        ctx.bot
            .edit_message_text(
                chat_id,
                message_id,
                format!("Đã huỷ lệnh hoàn tiền đơn {order_id}."),
            )
            .await?;
        return Ok(());
    }

    if let Some(order_id) = data.strip_prefix("admin_refund:confirm:") {
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(order) = repo::get_order_with_product(&ctx.pool, order_id).await? else {
            ctx.bot
                .edit_message_text(chat_id, message_id, "Không tìm thấy đơn để hoàn tiền.")
                .await?;
            return Ok(());
        };
        let username = order_user_display(ctx, order.order.user_id).await;
        let outcome = refund_paid_order_to_wallet(ctx, order_id, admin_id, &username).await?;
        let amount_line = match outcome.balance_after {
            Some(balance) => format!(
                "Đã cộng {} vào ví. Số dư sau hoàn: {}.",
                format_vnd(outcome.order.order.amount),
                format_vnd(balance)
            ),
            None => "Đơn này đã được hoàn tiền trước đó, không cộng thêm lần nữa.".to_string(),
        };
        ctx.bot
            .edit_message_text(
                chat_id,
                message_id,
                format!(
                    "Hoàn tiền thành công\n\nOrder: {}\nUser ID: {}\nUsername: {}\n{}",
                    outcome.order.order.id,
                    outcome.order.order.user_id,
                    outcome.username,
                    amount_line
                ),
            )
            .await?;
        return Ok(());
    }

    let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
    Ok(())
}

fn format_order_detail_text(ctx: &AppContext, lang: &str, order: &OrderWithProduct) -> String {
    let mut lines = vec![
        i18n::t(ctx, lang, "order_detail_title", "🧾 CHI TIẾT ĐƠN HÀNG"),
        String::new(),
        order_detail_line(
            ctx,
            lang,
            "order_detail_product_label",
            "Sản phẩm",
            &order.product.name,
        ),
    ];
    if !product_is_active(order) {
        lines.push(i18n::t(
            ctx,
            lang,
            "order_detail_inactive_product",
            "Trạng thái sản phẩm: Sản phẩm đã ngừng bán",
        ));
    }
    if let Some(plan_label) = order
        .order
        .plan_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(order_detail_line(
            ctx,
            lang,
            "order_detail_plan_label",
            "Gói",
            plan_label,
        ));
    }
    lines.extend([
        order_detail_line(
            ctx,
            lang,
            "order_detail_qty_label",
            "Số lượng",
            order.order.qty,
        ),
        order_detail_line(
            ctx,
            lang,
            "order_detail_amount_label",
            "Tổng tiền",
            format_vnd(order.order.amount),
        ),
        order_detail_line(
            ctx,
            lang,
            "order_detail_status_label",
            "Trạng thái",
            format_status(ctx, lang, &order.order.status),
        ),
        order_detail_line(
            ctx,
            lang,
            "order_detail_bank_memo_label",
            "Nội dung chuyển khoản",
            &order.order.bank_memo,
        ),
        order_detail_line(
            ctx,
            lang,
            "order_detail_paid_at_label",
            "Thời gian thanh toán",
            format_optional_vietnam_time(order.order.paid_at.as_deref()),
        ),
    ]);
    if let Some(tx_id) = order
        .order
        .payment_tx_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(order_detail_line(
            ctx,
            lang,
            "order_detail_tx_id_label",
            "Mã giao dịch",
            tx_id,
        ));
    }
    if let Some(customer_input) = order
        .order
        .customer_input
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(order_detail_line(
            ctx,
            lang,
            "order_detail_customer_input_label",
            "Thông tin đã nhập",
            customer_input,
        ));
    }
    if let Some(delivered_data) = order
        .order
        .delivered_data
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push(format!(
            "{}:",
            i18n::t(
                ctx,
                lang,
                "order_detail_delivered_data_label",
                "Dữ liệu giao hàng"
            )
        ));
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
                .reply_markup(orders_empty_keyboard(ctx, lang))
                .await?;
        } else {
            ctx.bot
                .send_message(chat_id, text)
                .reply_markup(orders_empty_keyboard(ctx, lang))
                .await?;
        }
        return Ok(());
    }
    let text = order_history_text(ctx, lang);
    let keyboard = order_history_keyboard(ctx, lang, &orders);
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

    if let Some(order_id) = data.strip_prefix("order_cookie:") {
        send_order_cookie(ctx, chat_id, order_id, user_id, &lang).await?;
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
                .reply_markup(order_not_found_keyboard(ctx, &lang))
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
            .reply_markup(order_not_found_keyboard(ctx, &lang))
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
            .reply_markup(order_not_found_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    };

    ctx.bot
        .edit_message_text(
            chat_id,
            message_id,
            format_order_detail_text(ctx, &lang, &order),
        )
        .reply_markup(order_detail_keyboard(ctx, &lang, &order))
        .await?;
    Ok(())
}

async fn send_order_cookie(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    order_id: &str,
    user_id: i64,
    lang: &str,
) -> anyhow::Result<()> {
    let Some(order) = repo::get_paid_order_for_user(&ctx.pool, order_id, user_id).await? else {
        ctx.bot
            .send_message(
                chat_id,
                ctx.get_text_lang("order_not_found", lang, "Order not found."),
            )
            .reply_markup(order_not_found_keyboard(ctx, lang))
            .await?;
        return Ok(());
    };

    let delivered_data = order
        .order
        .delivered_data
        .as_deref()
        .unwrap_or_default();
    let deliveries = parse_account_delivery_items(delivered_data);
    let Some(cookie_text) = format_cookie_text(&deliveries) else {
        ctx.bot
            .send_message(chat_id, "Đơn này không có cookie.")
            .reply_markup(order_detail_keyboard(ctx, lang, &order))
            .await?;
        return Ok(());
    };

    if cookie_text.chars().count() <= 3500 {
        ctx.bot
            .send_message(chat_id, cookie_message_html(&cookie_text))
            .parse_mode(ParseMode::Html)
            .reply_markup(order_detail_keyboard(ctx, lang, &order))
            .await?;
    } else {
        ctx.bot
            .send_document(
                chat_id,
                InputFile::memory(cookie_text.into_bytes())
                    .file_name(format!("cookie_{}.txt", order.order.bank_memo)),
            )
            .caption("Cookie của đơn hàng được gửi trong file.")
            .reply_markup(order_detail_keyboard(ctx, lang, &order))
            .await?;
    }

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
                    .send_message(
                        msg.chat.id,
                        i18n::t(&ctx, &lang, "order_usage_text", "Dùng: /order <order_id>"),
                    )
                    .reply_markup(orders_empty_keyboard(&ctx, &lang))
                    .await?;
                return Ok(true);
            }
            if let Some(order) =
                repo::get_paid_order_for_user(&ctx.pool, &order_id, user.id.0 as i64).await?
            {
                ctx.bot
                    .send_message(msg.chat.id, format_order_detail_text(&ctx, &lang, &order))
                    .reply_markup(order_detail_keyboard(&ctx, &lang, &order))
                    .await?;
            } else {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        ctx.get_text_lang("order_not_found", &lang, "Order not found."),
                    )
                    .reply_markup(order_not_found_keyboard(&ctx, &lang))
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
        if data.starts_with("admin_refund:") || data.starts_with("admin_order:") {
            handle_admin_refund_callback(&ctx, q).await?;
            return Ok(true);
        }

        if data == "orders:list"
            || data.starts_with("order:")
            || data.starts_with("order_cookie:")
            || data.starts_with("order_support:")
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
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use crate::domains::orders::models::{Order, OrderReservationMode, OrderWithProduct};
    use crate::domains::products::models::Product;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::HashMap;
    use teloxide::Bot;

    fn test_ctx_with_texts(texts: HashMap<String, String>) -> Arc<AppContext> {
        let pool = SqlitePoolOptions::new()
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
                crypto: crate::config::CryptoConfig::default(),
            },
            HashMap::new(),
            BotTexts::from_map(texts),
            vec![],
        )
    }

    fn test_ctx() -> Arc<AppContext> {
        test_ctx_with_texts(HashMap::new())
    }

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
                show_sold_count: Some(0),
            },
        }
    }

    #[tokio::test]
    async fn order_history_keyboard_labels_paid_orders_by_bank_memo() {
        let paid = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let pending = test_order("order-pending-1", OrderStatus::Pending, "DHPEND1234");
        let ctx = test_ctx();

        let keyboard = order_history_keyboard(&ctx, "vi", &[paid, pending]);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["callback_data"], "order:order-paid-1");
        let label = rows[0][0]["text"].as_str().unwrap();
        assert!(label.contains("DHPAID1234"));
        assert!(label.contains("24/05"));
        assert!(!label.contains("order-paid-1"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn order_history_button_template_strips_product_custom_emoji_placeholders() {
        let mut paid = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        paid.product.name = "{6089306715604392889} Acc FB long product".to_string();
        let ctx = test_ctx_with_texts(HashMap::from([(
            "orders_history_button_vi".to_string(),
            "{memo} • {date} • {product}".to_string(),
        )]));

        let label = order_button_label(&ctx, "vi", &paid);

        assert_eq!(label, "DHPAID1234 • 24/05 • Acc FB long product");
        assert!(!label.contains("{6089306715604392889}"));
    }

    #[tokio::test]
    async fn order_history_keyboard_uses_back_label_for_shop_return() {
        let paid = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let ctx = test_ctx();

        let keyboard = order_history_keyboard(&ctx, "vi", &[paid]);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["text"], "⬅️ Quay lại");
        assert_eq!(last_row[0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn orders_empty_keyboard_has_back_to_shop() {
        let ctx = test_ctx();
        let keyboard = orders_empty_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:shop"));
    }

    #[tokio::test]
    async fn orders_empty_keyboard_uses_admin_text() {
        let ctx = test_ctx_with_texts(HashMap::from([(
            "orders_empty_shop_btn_vi".to_string(),
            "🛍️ Đi shop".to_string(),
        )]));
        let keyboard = orders_empty_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();

        assert_eq!(json["inline_keyboard"][0][0]["text"], "🛍️ Đi shop");
    }

    #[tokio::test]
    async fn order_history_text_uses_admin_text() {
        let ctx = test_ctx_with_texts(HashMap::from([(
            "orders_history_text_vi".to_string(),
            "Lịch sử tùy chỉnh".to_string(),
        )]));

        assert_eq!(order_history_text(&ctx, "vi"), "Lịch sử tùy chỉnh");
    }

    #[tokio::test]
    async fn order_detail_text_hides_internal_fields_and_formats_paid_time() {
        let order = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let ctx = test_ctx();

        let text = format_order_detail_text(&ctx, "vi", &order);

        assert!(!text.contains("order-paid-1"));
        assert!(!text.contains("Mã đơn"));
        assert!(!text.contains("Thời gian tạo"));
        assert!(!text.contains("2026-05-24T10:00:00Z"));
        assert!(text.contains("Gói VIP"));
        assert!(text.contains("2"));
        assert!(text.contains("120.000đ"));
        assert!(text.contains("paid"));
        assert!(text.contains("DHPAID1234"));
        assert!(text.contains("24/05/2026 17:02:00"));
        assert!(!text.contains("2026-05-24T10:02:00Z"));
        assert!(text.contains("TX123"));
        assert!(text.contains("license-key-1"));
        assert!(text.contains("buyer@example.com"));
    }

    #[tokio::test]
    async fn order_detail_keyboard_allows_rebuy_only_for_active_products() {
        let active = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        let mut inactive = active.clone();
        inactive.product.is_active = Some(0);
        let ctx = test_ctx();

        let active_keyboard = order_detail_keyboard(&ctx, "vi", &active);
        let active_json = serde_json::to_value(&active_keyboard).unwrap();
        let active_rows = active_json["inline_keyboard"].as_array().unwrap();
        assert_eq!(active_rows[0][0]["callback_data"], "buy:1");

        let inactive_keyboard = order_detail_keyboard(&ctx, "vi", &inactive);
        let inactive_json = serde_json::to_value(&inactive_keyboard).unwrap();
        let inactive_rows = inactive_json["inline_keyboard"].as_array().unwrap();
        let callbacks = inactive_rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();
        assert!(!callbacks.contains(&"buy:1"));
    }

    #[tokio::test]
    async fn order_detail_text_marks_inactive_product_as_unavailable() {
        let mut order = test_order("order-paid-1", OrderStatus::Paid, "DHPAID1234");
        order.product.is_active = Some(0);
        let ctx = test_ctx();

        let text = format_order_detail_text(&ctx, "vi", &order);

        assert!(text.contains("Sản phẩm đã ngừng bán"));
    }
}
