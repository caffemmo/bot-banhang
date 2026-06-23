use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message,
};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::BotDialogue;
use crate::core::time::format_vietnam_time;

pub struct TutDeleteCommandPlugin;

const TUT_LIST: &str = "tut:list";
const TUT_DELETE_PREFIX: &str = "tutdel:ask:";
const TUT_DELETE_CONFIRM_PREFIX: &str = "tutdel:confirm:";
const TUT_DELETE_CANCEL: &str = "tutdel:cancel";

#[derive(Debug, Clone, FromRow)]
struct TutRow {
    id: i64,
    title: String,
    access_type: String,
    is_active: i64,
    view_count: i64,
    created_by: i64,
    created_at: String,
}

#[async_trait::async_trait]
impl AppPlugin for TutDeleteCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdTutDelete"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "tutdel".to_string(),
            description: "Admin: delete TUT".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        let user_id = msg.from().map(|user| user.id.0 as i64).unwrap_or(0);

        if is_command(text, "/tutlist") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot
                    .send_message(msg.chat.id, "Bạn không có quyền xem TUT.")
                    .await?;
                return Ok(true);
            }
            send_tut_list_with_delete(&ctx, msg.chat.id).await?;
            return Ok(true);
        }

        if is_command(text, "/tutdel") || is_command(text, "/tutdelete") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot
                    .send_message(msg.chat.id, "Bạn không có quyền xóa TUT.")
                    .await?;
                return Ok(true);
            }
            handle_tutdelete_command(&ctx, msg.chat.id, text).await?;
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
        let Some(data) = q.data.clone() else {
            return Ok(false);
        };
        if !data.starts_with("tutdel:") && data != TUT_LIST {
            return Ok(false);
        }

        let user_id = q.from.id.0 as i64;
        let chat_id = q.message.as_ref().map(|msg| msg.chat().id);
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;

        if !is_tut_admin(&ctx, user_id) {
            if let Some(chat_id) = chat_id {
                ctx.bot
                    .send_message(chat_id, "Bạn không có quyền quản lý TUT.")
                    .await?;
            }
            return Ok(true);
        }

        if data == TUT_LIST || data == TUT_DELETE_CANCEL {
            if let Some(chat_id) = chat_id {
                send_tut_list_with_delete(&ctx, chat_id).await?;
            }
            return Ok(true);
        }

        if let Some(id) = data
            .strip_prefix(TUT_DELETE_PREFIX)
            .and_then(|value| value.parse::<i64>().ok())
        {
            if let Some(chat_id) = chat_id {
                confirm_delete_tut(&ctx, chat_id, id).await?;
            }
            return Ok(true);
        }

        if let Some(id) = data
            .strip_prefix(TUT_DELETE_CONFIRM_PREFIX)
            .and_then(|value| value.parse::<i64>().ok())
        {
            if let Some(chat_id) = chat_id {
                delete_tut_and_refresh(&ctx, chat_id, id).await?;
            }
            return Ok(true);
        }

        Ok(false)
    }
}

async fn send_tut_list_with_delete(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let rows = list_active_tuts(&ctx.pool, 20).await?;
    if rows.is_empty() {
        ctx.bot
            .send_message(
                chat_id,
                "Chưa có TUT nào đang bật. Dùng /tutadd để tạo TUT đầu tiên.",
            )
            .await?;
        return Ok(());
    }

    let mut text = vec!["📚 DANH SÁCH TUT".to_string(), String::new()];
    for tut in &rows {
        text.push(format!(
            "#{} | {} | {} | xem {} | active={} | tạo bởi {} | {}",
            tut.id,
            tut_access_label(&tut.access_type),
            tut.title,
            tut.view_count,
            tut.is_active,
            tut.created_by,
            format_vietnam_time(&tut.created_at)
        ));
    }

    let mut buttons = Vec::new();
    for tut in rows.into_iter().take(8) {
        buttons.push(vec![
            InlineKeyboardButton::callback(
                format!("👀 #{} {}", tut.id, short_label(&tut.title, 18)),
                format!("tut:view:{}", tut.id),
            ),
            InlineKeyboardButton::callback("📤 Kênh", format!("tut:post:{}", tut.id)),
        ]);
        buttons.push(vec![
            InlineKeyboardButton::callback("📣 Bot", format!("tut:broadcast:{}", tut.id)),
            InlineKeyboardButton::callback("🗑 Xóa", format!("{}{}", TUT_DELETE_PREFIX, tut.id)),
        ]);
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ Menu TUT", "tut:home")]);

    ctx.bot
        .send_message(chat_id, text.join("\n"))
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;
    Ok(())
}

async fn handle_tutdelete_command(ctx: &AppContext, chat_id: ChatId, text: &str) -> Result<()> {
    let mut parts = text.split_whitespace();
    let _ = parts.next();
    let Some(id) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
        ctx.bot
            .send_message(chat_id, "Cách dùng: /tutdel <id>\nVí dụ: /tutdel 12")
            .await?;
        return Ok(());
    };
    confirm_delete_tut(ctx, chat_id, id).await
}

async fn confirm_delete_tut(ctx: &AppContext, chat_id: ChatId, id: i64) -> Result<()> {
    let Some(tut) = get_active_tut(&ctx.pool, id).await? else {
        ctx.bot
            .send_message(chat_id, "Không tìm thấy TUT hoặc TUT đã bị xóa.")
            .await?;
        return Ok(());
    };

    ctx.bot
        .send_message(
            chat_id,
            format!(
                "⚠️ Xác nhận xóa TUT #{}\n\n{}\n\nSau khi xóa, user sẽ không xem được TUT này nữa.",
                tut.id, tut.title
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "✅ Xóa thật",
                format!("{}{}", TUT_DELETE_CONFIRM_PREFIX, tut.id),
            )],
            vec![InlineKeyboardButton::callback("⬅️ Hủy", TUT_DELETE_CANCEL)],
        ]))
        .await?;
    Ok(())
}

async fn delete_tut_and_refresh(ctx: &AppContext, chat_id: ChatId, id: i64) -> Result<()> {
    let deleted = soft_delete_tut(&ctx.pool, id).await?;
    if deleted == 0 {
        ctx.bot
            .send_message(chat_id, "TUT này đã bị xóa hoặc không tồn tại.")
            .await?;
        return Ok(());
    }

    ctx.bot
        .send_message(chat_id, format!("✅ Đã xóa TUT #{} khỏi danh sách.", id))
        .await?;
    send_tut_list_with_delete(ctx, chat_id).await
}

async fn list_active_tuts(pool: &SqlitePool, limit: i64) -> Result<Vec<TutRow>> {
    sqlx::query_as::<_, TutRow>(
        r#"
        SELECT id, title, access_type, is_active, view_count, created_by, created_at
        FROM vip_tuts
        WHERE is_active = 1
        ORDER BY id DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn get_active_tut(pool: &SqlitePool, id: i64) -> Result<Option<TutRow>> {
    sqlx::query_as::<_, TutRow>(
        r#"
        SELECT id, title, access_type, is_active, view_count, created_by, created_at
        FROM vip_tuts
        WHERE id = ? AND is_active = 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn soft_delete_tut(pool: &SqlitePool, id: i64) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE vip_tuts SET is_active = 0, updated_at = ? WHERE id = ? AND is_active = 1",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn is_tut_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}

fn is_command(text: &str, command: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    first == command || first.starts_with(&format!("{command}@"))
}

fn tut_access_label(access_type: &str) -> &'static str {
    if access_type.eq_ignore_ascii_case("free") {
        "FREE"
    } else {
        "VIP"
    }
}

fn short_label(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push('…');
    }
    out
}
