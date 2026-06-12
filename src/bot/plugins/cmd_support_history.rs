use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use sqlx::FromRow;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::requests::Requester;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardMarkup, Message};

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::domains::orders::repo as orders_repo;

const HISTORY_CALLBACK: &str = "support_history:list";
const SUPPORT_CALLBACK_PREFIX: &str = "order_support:";
const HISTORY_LIMIT: i64 = 30;
const HISTORY_RENDER_LIMIT: usize = 10;

pub struct SupportHistoryCommandPlugin;

#[derive(Debug, FromRow)]
struct SupportCallbackLog {
    id: i64,
    user_id: Option<i64>,
    raw_json: String,
    created_at: Option<String>,
}

#[async_trait::async_trait]
impl AppPlugin for SupportHistoryCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdSupportHistory"
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(text) = msg.text() else {
            return Ok(false);
        };
        if !is_plain_start_command(text) {
            return Ok(false);
        }

        let Some(user) = msg.from() else {
            return Ok(false);
        };
        let user_id = user.id.0 as i64;
        if !is_support_history_admin(&ctx, user_id) {
            return Ok(false);
        }

        let lang = i18n::user_lang(&ctx, user_id, user.language_code.as_deref()).await;
        dialogue.update(State::Idle).await?;
        send_admin_start_menu(&ctx, msg.chat.id, &lang).await?;
        Ok(true)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(data) = q.data.as_deref() else {
            return Ok(false);
        };
        if data != HISTORY_CALLBACK && data != "start:menu" {
            return Ok(false);
        }

        let admin_id = q.from.id.0 as i64;
        if !is_support_history_admin(&ctx, admin_id) {
            if data == HISTORY_CALLBACK {
                let _ = ctx
                    .bot
                    .answer_callback_query(q.id.clone())
                    .text("Bạn không có quyền xem lịch sử yêu cầu hỗ trợ.")
                    .show_alert(true)
                    .await;
                return Ok(true);
            }
            return Ok(false);
        }

        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(ref msg) = q.message else {
            return Ok(true);
        };
        let lang = i18n::user_lang(&ctx, admin_id, q.from.language_code.as_deref()).await;
        if data == "start:menu" {
            send_admin_start_menu(&ctx, msg.chat().id, &lang).await?;
        } else {
            show_support_history(&ctx, msg.chat().id, &lang).await?;
        }
        Ok(true)
    }
}

async fn send_admin_start_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let text = i18n::t(
        ctx,
        lang,
        "start",
        "Welcome! Use the buttons below to browse products, view orders, or get help.",
    );
    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(admin_start_keyboard(ctx, lang))
        .await?;
    Ok(())
}

fn admin_start_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_shop",
            "🛒 Xem sản phẩm",
            "start:shop",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_orders",
            "🧾 Lịch sử mua hàng",
            "orders:list",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_wallet",
            "💰 Ví của tôi",
            "wallet:menu",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_help",
            "❓ Hỗ trợ",
            "start:help",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_support_history",
            "📋 Lịch sử yêu cầu",
            HISTORY_CALLBACK,
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_language",
            "🌐 Language / Ngôn ngữ",
            "start:language",
        )],
    ])
}

async fn show_support_history(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let logs = list_support_callback_logs(ctx, HISTORY_LIMIT).await?;
    let mut lines = vec!["📋 LỊCH SỬ YÊU CẦU HỖ TRỢ".to_string()];
    let mut rendered = 0usize;

    for log in logs {
        if rendered >= HISTORY_RENDER_LIMIT {
            break;
        }
        let Some(snapshot) = SupportRequestSnapshot::from_log(&log) else {
            continue;
        };

        let order = orders_repo::get_order_with_product(&ctx.pool, &snapshot.order_id)
            .await
            .ok()
            .flatten();
        lines.push(String::new());
        lines.push(format!("#{} - {}", log.id, display_time(log.created_at.as_deref())));
        lines.push(format!(
            "User: {}",
            snapshot
                .username
                .as_deref()
                .map(format_username)
                .unwrap_or_else(|| snapshot.user_id.to_string())
        ));
        lines.push(format!("Order: {}", snapshot.order_id));
        if let Some(order) = order {
            lines.push(format!("Memo: {}", order.order.bank_memo));
            lines.push(format!("Sản phẩm: {}", truncate_chars(&order.product.name, 44)));
            lines.push(format!("Số tiền: {}", format_vnd(order.order.amount)));
        } else {
            lines.push("Đơn hàng: không tìm thấy hoặc đã bị xoá".to_string());
        }
        rendered += 1;
    }

    if rendered == 0 {
        lines.push(String::new());
        lines.push("Chưa có yêu cầu hỗ trợ nào.".to_string());
    }

    ctx.bot
        .send_message(chat_id, lines.join("\n"))
        .reply_markup(back_to_start_keyboard(ctx, lang))
        .await?;
    Ok(())
}

async fn list_support_callback_logs(
    ctx: &AppContext,
    limit: i64,
) -> Result<Vec<SupportCallbackLog>> {
    let rows = sqlx::query_as::<_, SupportCallbackLog>(
        r#"
        SELECT id, user_id, raw_json, created_at
        FROM telegram_update_logs
        WHERE update_type = 'callback_query'
          AND raw_json LIKE ?1
        ORDER BY id DESC
        LIMIT ?2
        "#,
    )
    .bind(format!("%{SUPPORT_CALLBACK_PREFIX}%"))
    .bind(limit)
    .fetch_all(&ctx.pool)
    .await?;

    Ok(rows)
}

fn back_to_start_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![i18n::inline_button_callback(
        ctx,
        lang,
        "support_history_back_start_btn",
        "⬅️ Quay lại /start",
        "start:menu",
    )]])
}

fn is_support_history_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}

fn is_plain_start_command(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    trimmed == "/start" || trimmed.starts_with("/start@")
}

#[derive(Debug)]
struct SupportRequestSnapshot {
    order_id: String,
    user_id: i64,
    username: Option<String>,
}

impl SupportRequestSnapshot {
    fn from_log(log: &SupportCallbackLog) -> Option<Self> {
        let raw: Value = serde_json::from_str(&log.raw_json).ok()?;
        let order_id = raw
            .get("data")
            .and_then(Value::as_str)?
            .strip_prefix(SUPPORT_CALLBACK_PREFIX)?
            .to_string();
        let from = raw.get("from");
        let user_id = from
            .and_then(|from| from.get("id"))
            .and_then(Value::as_i64)
            .or(log.user_id)?;
        let username = from
            .and_then(|from| from.get("username"))
            .and_then(Value::as_str)
            .map(str::to_string);

        Some(Self {
            order_id,
            user_id,
            username,
        })
    }
}

fn format_username(username: &str) -> String {
    if username.starts_with('@') {
        username.to_string()
    } else {
        format!("@{username}")
    }
}

fn display_time(value: Option<&str>) -> &str {
    value.unwrap_or("không rõ thời gian")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut output = trimmed.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    output.push('…');
    output
}

fn format_vnd(amount: i64) -> String {
    let raw = amount.abs().to_string();
    let mut grouped = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut value = grouped.chars().rev().collect::<String>();
    if amount < 0 {
        value.insert(0, '-');
    }
    format!("{value} VND")
}
