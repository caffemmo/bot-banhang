use std::sync::Arc;

use anyhow::Result;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message};

use crate::app::AppContext;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::plugins::AppPlugin;
use crate::bot::BotDialogue;

pub struct TutPublicCommandPlugin;

const TUT_PUBLIC_HOME: &str = "tut:user_home";
const TUT_PUBLIC_FREE: &str = "tut:free";
const TUT_PUBLIC_VIP: &str = "tut:vip";
const TUT_PUBLIC_ALL: &str = "tut:all";
const TUT_MYVIP: &str = "tut:myvip";

#[derive(Debug, Clone, FromRow)]
struct PublicTut {
    id: i64,
    title: String,
    access_type: String,
}

#[async_trait::async_trait]
impl AppPlugin for TutPublicCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdTutPublic"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "tut".to_string(),
            description: "Kho TUT Free/VIP".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_command(text, "/tut") {
            return Ok(false);
        }

        let user_id = msg.from().map(|user| user.id.0 as i64).unwrap_or(0);
        if is_tut_admin(&ctx, user_id) {
            return Ok(false);
        }

        send_tut_public_home(&ctx, msg.chat.id).await?;
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

        let user_id = q.from.id.0 as i64;
        let chat_id = q.message.as_ref().map(|msg| msg.chat().id);

        match data {
            TUT_PUBLIC_HOME => {
                let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
                if let Some(chat_id) = chat_id {
                    send_tut_public_home(&ctx, chat_id).await?;
                }
                Ok(true)
            }
            TUT_PUBLIC_FREE => {
                let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
                if let Some(chat_id) = chat_id {
                    send_public_tut_list(&ctx, chat_id, user_id, Some("free")).await?;
                }
                Ok(true)
            }
            TUT_PUBLIC_VIP => {
                let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
                if let Some(chat_id) = chat_id {
                    send_public_tut_list(&ctx, chat_id, user_id, Some("vip")).await?;
                }
                Ok(true)
            }
            TUT_PUBLIC_ALL => {
                let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
                if let Some(chat_id) = chat_id {
                    send_public_tut_list(&ctx, chat_id, user_id, None).await?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

async fn send_tut_public_home(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let text = format!(
        "📚 KHO TUT MMO\n\n\
        Chọn mục bạn muốn xem:\n\n\
        🆓 TUT miễn phí: xem được ngay.\n\
        👑 TUT VIP: cần VIP còn hạn.\n\n\
        Gói VIP TUT: {} / {} ngày",
        format_vnd(vip_price(ctx)),
        vip_days(ctx)
    );

    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![
                InlineKeyboardButton::callback("🆓 TUT miễn phí", TUT_PUBLIC_FREE),
                InlineKeyboardButton::callback("👑 TUT VIP", TUT_PUBLIC_VIP),
            ],
            vec![InlineKeyboardButton::callback("📚 Tất cả TUT", TUT_PUBLIC_ALL)],
            vec![
                InlineKeyboardButton::callback("💎 Mua VIP TUT", "tut:buyvip"),
                InlineKeyboardButton::callback("🔎 Kiểm tra VIP", TUT_MYVIP),
            ],
        ]))
        .await?;
    Ok(())
}

async fn send_public_tut_list(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    access_type: Option<&str>,
) -> Result<()> {
    let rows = list_public_tuts(&ctx.pool, access_type, 20).await?;
    if rows.is_empty() {
        let label = match access_type {
            Some("free") => "TUT miễn phí",
            Some("vip") => "TUT VIP",
            _ => "TUT",
        };
        ctx.bot
            .send_message(chat_id, format!("Hiện chưa có {label} nào."))
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "⬅️ Menu TUT",
                TUT_PUBLIC_HOME,
            )]]))
            .await?;
        return Ok(());
    }

    let title = match access_type {
        Some("free") => "🆓 TUT MIỄN PHÍ",
        Some("vip") => "👑 TUT VIP",
        _ => "📚 TẤT CẢ TUT",
    };
    let vip_active = if user_id > 0 {
        vip_is_active(&ctx.pool, user_id).await?
    } else {
        false
    };

    let mut text = vec![title.to_string(), String::new()];
    for tut in &rows {
        let access_icon = if tut_is_free(tut) { "🆓" } else { "👑" };
        let status = if tut_is_free(tut) || vip_active || is_tut_admin(ctx, user_id) {
            "xem được"
        } else {
            "cần VIP"
        };
        text.push(format!(
            "{access_icon} #{} | {} | {}",
            tut.id,
            short_label(&tut.title, 42),
            status
        ));
    }

    let mut buttons = Vec::new();
    for chunk in rows.chunks(2) {
        buttons.push(
            chunk
                .iter()
                .map(|tut| {
                    let icon = if tut_is_free(tut) { "🆓" } else { "👑" };
                    InlineKeyboardButton::callback(
                        format!("{icon} {}", short_label(&tut.title, 28)),
                        format!("tut:view:{}", tut.id),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    buttons.push(vec![
        InlineKeyboardButton::callback("💎 Mua VIP", "tut:buyvip"),
        InlineKeyboardButton::callback("🔎 Kiểm tra VIP", TUT_MYVIP),
    ]);
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ Menu TUT", TUT_PUBLIC_HOME)]);

    ctx.bot
        .send_message(chat_id, text.join("\n"))
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;
    Ok(())
}

async fn list_public_tuts(
    pool: &SqlitePool,
    access_type: Option<&str>,
    limit: i64,
) -> Result<Vec<PublicTut>> {
    if let Some(access_type) = access_type {
        sqlx::query_as::<_, PublicTut>(
            r#"
            SELECT id, title, access_type
            FROM vip_tuts
            WHERE is_active = 1 AND lower(access_type) = lower(?)
            ORDER BY id DESC
            LIMIT ?
            "#,
        )
        .bind(access_type)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
    } else {
        sqlx::query_as::<_, PublicTut>(
            r#"
            SELECT id, title, access_type
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
}

async fn vip_is_active(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let now = chrono::Utc::now().to_rfc3339();
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM vip_tut_memberships WHERE user_id = ? AND expires_at > ?",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

fn vip_price(ctx: &AppContext) -> i64 {
    ctx.get_text("vip_tut_price", "99000")
        .trim()
        .parse::<i64>()
        .unwrap_or(99_000)
}

fn vip_days(ctx: &AppContext) -> i64 {
    ctx.get_text("vip_tut_days", "30")
        .trim()
        .parse::<i64>()
        .unwrap_or(30)
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

fn tut_is_free(tut: &PublicTut) -> bool {
    tut.access_type.eq_ignore_ascii_case("free")
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
