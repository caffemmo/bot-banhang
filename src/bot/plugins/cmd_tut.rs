use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message,
};
use tokio::time::{sleep, Duration as TokioDuration};
use url::Url;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::{BotDialogue, State};
use crate::domains::users::repo as users_repo;
use crate::domains::wallet::repo as wallet_repo;

pub struct TutCommandPlugin;

const TUT_PREFIX: &str = "tut:";
const TUT_HOME: &str = "tut:home";
const TUT_ADD: &str = "tut:add";
const TUT_LIST: &str = "tut:list";
const TUT_MYVIP: &str = "tut:myvip";
const TUT_BROADCAST_PREFIX: &str = "tut:broadcast:";
const TUT_BROADCAST_CONFIRM_PREFIX: &str = "tut:broadcast_confirm:";
const TUT_BROADCAST_CANCEL: &str = "tut:broadcast_cancel";

#[derive(Debug, Clone, FromRow)]
struct VipTut {
    id: i64,
    title: String,
    teaser: String,
    content: String,
    access_type: String,
    is_active: i64,
    view_count: i64,
    created_by: i64,
    created_at: String,
    updated_at: String,
    posted_at: Option<String>,
    posted_chat_id: Option<String>,
    posted_message_id: Option<i64>,
}

#[async_trait::async_trait]
impl AppPlugin for TutCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdTut"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![
            BotCommand {
                command: "tut".to_string(),
                description: "Admin: manage TUT".to_string(),
            },
            BotCommand {
                command: "tutadd".to_string(),
                description: "Admin: add TUT".to_string(),
            },
            BotCommand {
                command: "tutlist".to_string(),
                description: "Admin: list TUT".to_string(),
            },
            BotCommand {
                command: "tutpost".to_string(),
                description: "Admin: post TUT teaser".to_string(),
            },
            BotCommand {
                command: "myvip".to_string(),
                description: "View VIP TUT status".to_string(),
            },
            BotCommand {
                command: "tutvipadd".to_string(),
                description: "Admin: grant VIP TUT".to_string(),
            },
            BotCommand {
                command: "tutviplist".to_string(),
                description: "Admin: list VIP TUT users".to_string(),
            },
            BotCommand {
                command: "tutstats".to_string(),
                description: "Admin: VIP TUT stats".to_string(),
            },
        ]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        let user_id = msg.from().map(|user| user.id.0 as i64).unwrap_or(0);

        match dialogue.get().await?.unwrap_or_default() {
            State::CreatingTutTitle => {
                handle_create_title(&ctx, &msg, dialogue, text).await?;
                return Ok(true);
            }
            State::CreatingTutTeaser { title } => {
                handle_create_teaser(&ctx, &msg, dialogue, title, text).await?;
                return Ok(true);
            }
            State::CreatingTutContent { title, teaser } => {
                handle_create_content(&ctx, &msg, dialogue, user_id, title, teaser, text).await?;
                return Ok(true);
            }
            _ => {}
        }

        if let Some(id) = parse_start_tut_payload(text) {
            show_tut_or_paywall(&ctx, msg.chat.id, user_id, id).await?;
            return Ok(true);
        }

        if is_command(text, "/myvip") {
            send_my_vip(&ctx, msg.chat.id, user_id).await?;
            return Ok(true);
        }

        if is_command(text, "/tut") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền admin TUT.").await?;
                return Ok(true);
            }
            send_tut_admin_menu(&ctx, msg.chat.id).await?;
            return Ok(true);
        }

        if is_command(text, "/tutadd") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền thêm TUT.").await?;
                return Ok(true);
            }
            start_create_tut(&ctx, msg.chat.id, dialogue).await?;
            return Ok(true);
        }

        if is_command(text, "/tutlist") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền xem TUT.").await?;
                return Ok(true);
            }
            send_tut_list(&ctx, msg.chat.id).await?;
            return Ok(true);
        }

        if is_command(text, "/tutpost") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền đăng TUT.").await?;
                return Ok(true);
            }
            handle_tutpost_command(&ctx, msg.chat.id, text).await?;
            return Ok(true);
        }

        if is_command(text, "/tutvipadd") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền cấp VIP TUT.").await?;
                return Ok(true);
            }
            handle_tutvipadd_command(&ctx, msg.chat.id, text).await?;
            return Ok(true);
        }

        if is_command(text, "/tutviplist") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền xem VIP TUT.").await?;
                return Ok(true);
            }
            send_vip_list(&ctx, msg.chat.id).await?;
            return Ok(true);
        }

        if is_command(text, "/tutstats") {
            if !is_tut_admin(&ctx, user_id) {
                ctx.bot.send_message(msg.chat.id, "Bạn không có quyền xem thống kê TUT.").await?;
                return Ok(true);
            }
            send_tut_stats(&ctx, msg.chat.id).await?;
            return Ok(true);
        }

        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(data) = q.data.clone() else {
            return Ok(false);
        };
        if !data.starts_with(TUT_PREFIX) {
            return Ok(false);
        }

        let user_id = q.from.id.0 as i64;
        let chat_id = q.message.as_ref().map(|msg| msg.chat().id);
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;

        match data.as_str() {
            TUT_HOME => {
                if let Some(chat_id) = chat_id {
                    if is_tut_admin(&ctx, user_id) {
                        send_tut_admin_menu(&ctx, chat_id).await?;
                    }
                }
                return Ok(true);
            }
            TUT_ADD => {
                if let Some(chat_id) = chat_id {
                    if is_tut_admin(&ctx, user_id) {
                        start_create_tut(&ctx, chat_id, dialogue).await?;
                    }
                }
                return Ok(true);
            }
            TUT_LIST => {
                if let Some(chat_id) = chat_id {
                    if is_tut_admin(&ctx, user_id) {
                        send_tut_list(&ctx, chat_id).await?;
                    }
                }
                return Ok(true);
            }
            TUT_MYVIP => {
                if let Some(chat_id) = chat_id {
                    send_my_vip(&ctx, chat_id, user_id).await?;
                }
                return Ok(true);
            }
            _ => {}
        }

        if let Some(id) = data.strip_prefix("tut:view:").and_then(|v| v.parse::<i64>().ok()) {
            if let Some(chat_id) = chat_id {
                show_tut_or_paywall(&ctx, chat_id, user_id, id).await?;
            }
            return Ok(true);
        }

        if let Some(id) = data.strip_prefix("tut:buy:").and_then(|v| v.parse::<i64>().ok()) {
            if let Some(chat_id) = chat_id {
                buy_vip_and_show_tut(&ctx, chat_id, user_id, Some(id)).await?;
            }
            return Ok(true);
        }

        if data == "tut:buyvip" {
            if let Some(chat_id) = chat_id {
                buy_vip_and_show_tut(&ctx, chat_id, user_id, None).await?;
            }
            return Ok(true);
        }

        if let Some(id) = data.strip_prefix("tut:post:").and_then(|v| v.parse::<i64>().ok()) {
            if let Some(chat_id) = chat_id {
                if is_tut_admin(&ctx, user_id) {
                    post_tut_to_configured_channel(&ctx, chat_id, id).await?;
                }
            }
            return Ok(true);
        }

        if let Some(id) = data
            .strip_prefix(TUT_BROADCAST_PREFIX)
            .and_then(|v| v.parse::<i64>().ok())
        {
            if let Some(chat_id) = chat_id {
                if is_tut_admin(&ctx, user_id) {
                    confirm_broadcast_tut_to_bot(&ctx, chat_id, id).await?;
                }
            }
            return Ok(true);
        }

        if let Some(id) = data
            .strip_prefix(TUT_BROADCAST_CONFIRM_PREFIX)
            .and_then(|v| v.parse::<i64>().ok())
        {
            if let Some(chat_id) = chat_id {
                if is_tut_admin(&ctx, user_id) {
                    broadcast_tut_to_bot_users(&ctx, chat_id, id).await?;
                }
            }
            return Ok(true);
        }

        if data == TUT_BROADCAST_CANCEL {
            if let Some(chat_id) = chat_id {
                if is_tut_admin(&ctx, user_id) {
                    ctx.bot.send_message(chat_id, "Đã hủy gửi TUT vào bot.").await?;
                }
            }
            return Ok(true);
        }

        Ok(false)
    }
}

async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS vip_tuts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            teaser TEXT NOT NULL,
            content TEXT NOT NULL,
            access_type TEXT NOT NULL DEFAULT 'vip',
            is_active INTEGER NOT NULL DEFAULT 1,
            view_count INTEGER NOT NULL DEFAULT 0,
            created_by INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            posted_at TEXT,
            posted_chat_id TEXT,
            posted_message_id INTEGER
        )
        "#,
    )
    .execute(pool)
    .await?;

    let has_access_type = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM pragma_table_info('vip_tuts') WHERE name = 'access_type'",
    )
    .fetch_one(pool)
    .await?;
    if has_access_type == 0 {
        sqlx::query("ALTER TABLE vip_tuts ADD COLUMN access_type TEXT NOT NULL DEFAULT 'vip'")
            .execute(pool)
            .await?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS vip_tut_memberships (
            user_id INTEGER PRIMARY KEY,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS vip_tut_views (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tut_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            viewed_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn start_create_tut(ctx: &AppContext, chat_id: ChatId, dialogue: BotDialogue) -> Result<()> {
    dialogue.update(State::CreatingTutTitle).await?;
    ctx.bot
        .send_message(
            chat_id,
            "📚 Tạo TUT\n\nBước 1/3: gửi tiêu đề TUT.\n\nGõ /cancel để hủy.",
        )
        .await?;
    Ok(())
}

async fn handle_create_title(
    ctx: &AppContext,
    msg: &Message,
    dialogue: BotDialogue,
    text: &str,
) -> Result<()> {
    if is_cancel(text) {
        dialogue.update(State::Idle).await?;
        ctx.bot.send_message(msg.chat.id, "Đã hủy tạo TUT.").await?;
        return Ok(());
    }
    if text.len() < 3 {
        ctx.bot.send_message(msg.chat.id, "Tiêu đề quá ngắn, gửi lại giúp mình.").await?;
        return Ok(());
    }
    dialogue
        .update(State::CreatingTutTeaser {
            title: text.to_string(),
        })
        .await?;
    ctx.bot
        .send_message(
            msg.chat.id,
            "Bước 2/3: gửi mô tả ngắn để đăng công khai lên kênh/nhóm.",
        )
        .await?;
    Ok(())
}

async fn handle_create_teaser(
    ctx: &AppContext,
    msg: &Message,
    dialogue: BotDialogue,
    title: String,
    text: &str,
) -> Result<()> {
    if is_cancel(text) {
        dialogue.update(State::Idle).await?;
        ctx.bot.send_message(msg.chat.id, "Đã hủy tạo TUT.").await?;
        return Ok(());
    }
    if text.len() < 5 {
        ctx.bot.send_message(msg.chat.id, "Teaser quá ngắn, gửi lại giúp mình.").await?;
        return Ok(());
    }
    dialogue
        .update(State::CreatingTutContent {
            title,
            teaser: text.to_string(),
        })
        .await?;
    ctx.bot
        .send_message(
            msg.chat.id,
            "Bước 3/3: gửi nội dung full TUT.\n\nMặc định là TUT VIP.\nNếu muốn user thường xem miễn phí, hãy để dòng đầu tiên là: free",
        )
        .await?;
    Ok(())
}

async fn handle_create_content(
    ctx: &AppContext,
    msg: &Message,
    dialogue: BotDialogue,
    user_id: i64,
    title: String,
    teaser: String,
    text: &str,
) -> Result<()> {
    if is_cancel(text) {
        dialogue.update(State::Idle).await?;
        ctx.bot.send_message(msg.chat.id, "Đã hủy tạo TUT.").await?;
        return Ok(());
    }
    let (content, access_type) = parse_tut_content_and_access(text);
    if content.len() < 10 {
        ctx.bot
            .send_message(msg.chat.id, "Nội dung full quá ngắn, gửi lại giúp mình.")
            .await?;
        return Ok(());
    }

    let tut_id = insert_tut(&ctx.pool, &title, &teaser, &content, access_type, user_id).await?;
    dialogue.update(State::Idle).await?;
    let access_label = tut_access_label(access_type);
    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã lưu TUT {} #{}\n\nTiêu đề: {}\n\nBạn có thể đăng teaser lên kênh ngay.",
                access_label, tut_id, title
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "📤 Đăng teaser lên kênh",
                format!("tut:post:{tut_id}"),
            )],
            vec![
                InlineKeyboardButton::callback("👀 Xem full", format!("tut:view:{tut_id}")),
                InlineKeyboardButton::callback("📚 Menu TUT", TUT_HOME),
            ],
        ]))
        .await?;
    Ok(())
}

async fn send_tut_admin_menu(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let text = format!(
        "📚 QUẢN LÝ TUT\n\n\
        Admin viết TUT trong bot, chọn FREE hoặc VIP, rồi bot đăng teaser ra kênh/nhóm. TUT FREE ai cũng xem được, TUT VIP cần VIP còn hạn.\n\n\
        Giá VIP hiện tại: {}\n\
        Thời hạn: {} ngày\n\n\
        Lệnh nhanh:\n\
        /tutadd - thêm TUT, muốn FREE thì dòng đầu nội dung ghi free\n\
        /tutlist - danh sách TUT\n\
        /tutpost <id> - đăng teaser lên kênh cấu hình\n\
        /tutvipadd <telegram_id> [ngày] - cấp VIP thủ công\n\
        /tutviplist - danh sách VIP còn hạn\n\
        /tutstats - thống kê TUT/VIP\n\
        /myvip - xem hạn VIP",
        format_vnd(vip_price(ctx)),
        vip_days(ctx)
    );
    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![
                InlineKeyboardButton::callback("➕ Thêm TUT", TUT_ADD),
                InlineKeyboardButton::callback("📚 Danh sách", TUT_LIST),
            ],
            vec![InlineKeyboardButton::callback("👑 Xem VIP của tôi", TUT_MYVIP)],
        ]))
        .await?;
    Ok(())
}

async fn send_tut_list(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let rows = list_tuts(&ctx.pool, 20).await?;
    if rows.is_empty() {
        ctx.bot
            .send_message(chat_id, "Chưa có TUT nào. Dùng /tutadd để tạo TUT đầu tiên.")
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
            tut.created_at
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
        buttons.push(vec![InlineKeyboardButton::callback(
            "📣 Bot",
            format!("{}{}", TUT_BROADCAST_PREFIX, tut.id),
        )]);
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ Menu TUT", TUT_HOME)]);

    ctx.bot
        .send_message(chat_id, text.join("\n"))
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;
    Ok(())
}

async fn handle_tutpost_command(ctx: &AppContext, chat_id: ChatId, text: &str) -> Result<()> {
    let mut parts = text.split_whitespace();
    let _ = parts.next();
    let Some(id) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
        ctx.bot
            .send_message(chat_id, "Cách dùng: /tutpost <id>\nVí dụ: /tutpost 12")
            .await?;
        return Ok(());
    };
    post_tut_to_configured_channel(ctx, chat_id, id).await
}

async fn handle_tutvipadd_command(ctx: &AppContext, chat_id: ChatId, text: &str) -> Result<()> {
    let mut parts = text.split_whitespace();
    let _ = parts.next();
    let Some(target_user_id) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
        ctx.bot
            .send_message(
                chat_id,
                "Cách dùng: /tutvipadd <telegram_id> [số_ngày]\nVí dụ: /tutvipadd 5919002786 30",
            )
            .await?;
        return Ok(());
    };
    let days = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| vip_days(ctx));

    let expires_at = grant_vip_days(&ctx.pool, target_user_id, days).await?;
    ctx.bot
        .send_message(
            chat_id,
            format!("✅ Đã cấp VIP TUT cho user {target_user_id}\nHạn đến: {expires_at}"),
        )
        .await?;
    let _ = ctx
        .bot
        .send_message(
            ChatId(target_user_id),
            format!("👑 Bạn đã được cấp VIP TUT.\nHạn đến: {expires_at}"),
        )
        .await;
    Ok(())
}

async fn send_vip_list(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let rows = list_active_vips(&ctx.pool, 30).await?;
    if rows.is_empty() {
        ctx.bot.send_message(chat_id, "Chưa có user VIP TUT nào còn hạn.").await?;
        return Ok(());
    }

    let mut lines = vec!["👑 VIP TUT CÒN HẠN".to_string(), String::new()];
    for (user_id, expires_at) in rows {
        lines.push(format!("• {user_id} - hết hạn {expires_at}"));
    }
    ctx.bot.send_message(chat_id, lines.join("\n")).await?;
    Ok(())
}

async fn send_tut_stats(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let total_tuts = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM vip_tuts")
        .fetch_one(&ctx.pool)
        .await?;
    let active_tuts =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM vip_tuts WHERE is_active = 1")
            .fetch_one(&ctx.pool)
            .await?;
    let active_vips = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM vip_tut_memberships WHERE expires_at > ?",
    )
    .bind(Utc::now().to_rfc3339())
    .fetch_one(&ctx.pool)
    .await?;
    let total_views = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM vip_tut_views")
        .fetch_one(&ctx.pool)
        .await?;

    ctx.bot
        .send_message(
            chat_id,
            format!(
                "📊 THỐNG KÊ TUT VIP\n\nTổng TUT: {total_tuts}\nTUT đang bật: {active_tuts}\nVIP còn hạn: {active_vips}\nTổng lượt xem full: {total_views}"
            ),
        )
        .await?;
    Ok(())
}

async fn post_tut_to_configured_channel(ctx: &AppContext, admin_chat_id: ChatId, id: i64) -> Result<()> {
    let Some(channel) = configured_tut_channel(ctx) else {
        ctx.bot
            .send_message(
                admin_chat_id,
                "Chưa cấu hình kênh đăng TUT.\n\nVào admin config thêm key: vip_tut_channel_id\nGiá trị ví dụ: @tenkenh hoặc -100xxxxxxxxxx",
            )
            .await?;
        return Ok(());
    };
    let Some(tut) = get_tut(&ctx.pool, id).await? else {
        ctx.bot.send_message(admin_chat_id, "Không tìm thấy TUT.").await?;
        return Ok(());
    };

    let me = ctx.bot.get_me().await?;
    let Some(username) = me.user.username else {
        ctx.bot
            .send_message(admin_chat_id, "Bot chưa có username nên không tạo được link xem full.")
            .await?;
        return Ok(());
    };
    let url = Url::parse(&format!("https://t.me/{username}?start=tut_{}", tut.id))?;
    let (badge, footer) = if tut_is_free(&tut) {
        ("🆓", "Mở full miễn phí ngay trong bot.")
    } else {
        ("🔒", "👑 Thành viên VIP xem full ngay trong bot.")
    };
    let text = format!("{badge} {}\n\n{}\n\n{footer}", tut.title, tut.teaser);
    let sent = ctx
        .bot
        .send_message(channel.clone(), text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "🚀 Xem full trong bot",
            url,
        )]]))
        .await?;
    mark_tut_posted(&ctx.pool, tut.id, &channel, sent.id.0 as i64).await?;
    ctx.bot
        .send_message(
            admin_chat_id,
            format!("✅ Đã đăng teaser TUT #{} lên {}", tut.id, channel),
        )
        .await?;
    Ok(())
}

async fn confirm_broadcast_tut_to_bot(ctx: &AppContext, admin_chat_id: ChatId, id: i64) -> Result<()> {
    let Some(tut) = get_tut(&ctx.pool, id).await? else {
        ctx.bot.send_message(admin_chat_id, "Không tìm thấy TUT.").await?;
        return Ok(());
    };
    let total_users = users_repo::list_subscribers(&ctx.pool)
        .await?
        .into_iter()
        .filter(|sub| sub.chat_id != 0 && sub.is_bot.unwrap_or(0) == 0)
        .count();

    ctx.bot
        .send_message(
            admin_chat_id,
            format!(
                "📣 Gửi teaser TUT #{} vào bot cho {} user?\n\n{}\n\n{}",
                tut.id, total_users, tut.title, tut.teaser
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                "✅ Gửi vào bot",
                format!("{}{}", TUT_BROADCAST_CONFIRM_PREFIX, tut.id),
            ),
            InlineKeyboardButton::callback("❌ Hủy", TUT_BROADCAST_CANCEL),
        ]]))
        .await?;
    Ok(())
}

async fn broadcast_tut_to_bot_users(ctx: &AppContext, admin_chat_id: ChatId, id: i64) -> Result<()> {
    let Some(tut) = get_tut(&ctx.pool, id).await? else {
        ctx.bot.send_message(admin_chat_id, "Không tìm thấy TUT.").await?;
        return Ok(());
    };

    let me = ctx.bot.get_me().await?;
    let Some(username) = me.user.username else {
        ctx.bot
            .send_message(admin_chat_id, "Bot chưa có username nên không tạo được link xem full.")
            .await?;
        return Ok(());
    };
    let url = Url::parse(&format!("https://t.me/{username}?start=tut_{}", tut.id))?;
    let (badge, footer) = if tut_is_free(&tut) {
        ("🆓", "Mở full miễn phí ngay trong bot.")
    } else {
        ("🔒", "👑 Thành viên VIP xem full ngay trong bot.")
    };
    let text = format!("{badge} {}\n\n{}\n\n{footer}", tut.title, tut.teaser);
    let markup = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
        "🚀 Xem full trong bot",
        url,
    )]]);

    let subscribers = users_repo::list_subscribers(&ctx.pool).await?;
    let recipients = subscribers
        .into_iter()
        .filter(|sub| sub.chat_id != 0 && sub.is_bot.unwrap_or(0) == 0)
        .collect::<Vec<_>>();

    if recipients.is_empty() {
        ctx.bot
            .send_message(admin_chat_id, "Chưa có user nào trong bot để gửi TUT.")
            .await?;
        return Ok(());
    }

    ctx.bot
        .send_message(
            admin_chat_id,
            format!("📣 Bắt đầu gửi TUT #{} cho {} user...", tut.id, recipients.len()),
        )
        .await?;

    let mut sent = 0usize;
    let mut failed = 0usize;
    for sub in recipients {
        let result = ctx
            .bot
            .send_message(ChatId(sub.chat_id), text.clone())
            .reply_markup(markup.clone())
            .await;
        if result.is_ok() {
            sent += 1;
        } else {
            failed += 1;
        }
        sleep(TokioDuration::from_millis(40)).await;
    }

    ctx.bot
        .send_message(
            admin_chat_id,
            format!(
                "✅ Đã gửi TUT #{} vào bot.\nThành công: {}\nLỗi: {}",
                tut.id, sent, failed
            ),
        )
        .await?;
    Ok(())
}

async fn show_tut_or_paywall(ctx: &AppContext, chat_id: ChatId, user_id: i64, tut_id: i64) -> Result<()> {
    if user_id == 0 {
        ctx.bot.send_message(chat_id, "Không nhận diện được tài khoản Telegram.").await?;
        return Ok(());
    }
    let Some(tut) = get_tut(&ctx.pool, tut_id).await? else {
        ctx.bot.send_message(chat_id, "TUT này không tồn tại hoặc đã bị xóa.").await?;
        return Ok(());
    };
    if !tut_is_free(&tut) && !vip_is_active(&ctx.pool, user_id).await? && !is_tut_admin(ctx, user_id) {
        send_tut_paywall(ctx, chat_id, &tut).await?;
        return Ok(());
    }
    record_tut_view(&ctx.pool, tut.id, user_id).await?;
    let expires = if tut_is_free(&tut) {
        "\n🆓 TUT miễn phí\n".to_string()
    } else {
        vip_expires_at(&ctx.pool, user_id)
            .await?
            .map(|value| format!("\n👑 VIP còn hạn đến: {value}\n"))
            .unwrap_or_default()
    };
    let badge = if tut_is_free(&tut) {
        "🆓 Đã mở TUT FREE"
    } else {
        "✅ Đã mở khóa TUT"
    };
    let mut request = ctx.bot.send_message(
        chat_id,
        format!("{badge}{expires}\n📘 {}\n\n{}", tut.title, tut.content),
    );
    if !tut_is_free(&tut) {
        request = request.reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("👑 Kiểm tra VIP", TUT_MYVIP),
        ]]));
    }
    request.await?;
    Ok(())
}

async fn send_tut_paywall(ctx: &AppContext, chat_id: ChatId, tut: &VipTut) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "🔒 TUT này dành cho thành viên VIP\n\n📘 {}\n\n👑 Gói VIP TUT:\n• Xem toàn bộ TUT trong {} ngày\n• TUT mới cập nhật liên tục\n• Không cần mua từng bài lẻ\n\nGiá: {}",
                tut.title,
                vip_days(ctx),
                format_vnd(vip_price(ctx)),
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "👑 Mua VIP 30 ngày bằng ví",
                format!("tut:buy:{}", tut.id),
            )],
            vec![InlineKeyboardButton::callback("💳 Ví tiền", "start:wallet")],
        ]))
        .await?;
    Ok(())
}

async fn buy_vip_and_show_tut(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    tut_id: Option<i64>,
) -> Result<()> {
    let price = vip_price(ctx);
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;
    if wallet.balance < price {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "❌ Số dư ví không đủ để mua VIP.\n\nSố dư: {}\nCần: {}\n\nVui lòng nạp ví rồi thử lại.",
                    format_vnd(wallet.balance),
                    format_vnd(price)
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "💰 Nạp ví",
                "wallet:topup",
            )]]))
            .await?;
        return Ok(());
    }

    let expires_at = extend_vip_with_wallet(ctx, user_id, price, vip_days(ctx)).await?;
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Mua VIP TUT thành công\n\n👑 Hạn VIP đến: {}\n💳 Số dư còn lại: {}",
                expires_at.0,
                format_vnd(expires_at.1),
            ),
        )
        .await?;

    if let Some(tut_id) = tut_id {
        show_tut_or_paywall(ctx, chat_id, user_id, tut_id).await?;
    }
    Ok(())
}

async fn send_my_vip(ctx: &AppContext, chat_id: ChatId, user_id: i64) -> Result<()> {
    if user_id == 0 {
        ctx.bot.send_message(chat_id, "Không nhận diện được tài khoản Telegram.").await?;
        return Ok(());
    }
    if let Some(expires_at) = vip_expires_at(&ctx.pool, user_id).await? {
        if vip_is_active(&ctx.pool, user_id).await? {
            ctx.bot
                .send_message(chat_id, format!("👑 VIP TUT của bạn còn hạn đến:\n{expires_at}"))
                .await?;
        } else {
            ctx.bot
                .send_message(
                    chat_id,
                    format!("⏳ VIP TUT của bạn đã hết hạn:\n{expires_at}"),
                )
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                    "🔁 Gia hạn VIP 30 ngày",
                    "tut:buyvip",
                )]]))
                .await?;
        }
    } else {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "Bạn chưa có VIP TUT.\n\nGiá: {} / {} ngày",
                    format_vnd(vip_price(ctx)),
                    vip_days(ctx)
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "👑 Mua VIP 30 ngày",
                "tut:buyvip",
            )]]))
            .await?;
    }
    Ok(())
}

async fn insert_tut(
    pool: &SqlitePool,
    title: &str,
    teaser: &str,
    content: &str,
    access_type: &str,
    created_by: i64,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO vip_tuts (title, teaser, content, access_type, created_by, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(title)
    .bind(teaser)
    .bind(content)
    .bind(access_type)
    .bind(created_by)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn get_tut(pool: &SqlitePool, id: i64) -> Result<Option<VipTut>> {
    sqlx::query_as::<_, VipTut>(
        r#"
        SELECT id, title, teaser, content, access_type, is_active, view_count, created_by, created_at, updated_at,
               posted_at, posted_chat_id, posted_message_id
        FROM vip_tuts
        WHERE id = ? AND is_active = 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn list_tuts(pool: &SqlitePool, limit: i64) -> Result<Vec<VipTut>> {
    sqlx::query_as::<_, VipTut>(
        r#"
        SELECT id, title, teaser, content, access_type, is_active, view_count, created_by, created_at, updated_at,
               posted_at, posted_chat_id, posted_message_id
        FROM vip_tuts
        ORDER BY id DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn mark_tut_posted(
    pool: &SqlitePool,
    id: i64,
    chat_id: &str,
    message_id: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE vip_tuts SET posted_at = ?, posted_chat_id = ?, posted_message_id = ?, updated_at = ? WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(chat_id)
    .bind(message_id)
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn record_tut_view(pool: &SqlitePool, tut_id: i64, user_id: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO vip_tut_views (tut_id, user_id, viewed_at) VALUES (?, ?, ?)")
        .bind(tut_id)
        .bind(user_id)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE vip_tuts SET view_count = view_count + 1, updated_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(tut_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn vip_expires_at(pool: &SqlitePool, user_id: i64) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT expires_at FROM vip_tut_memberships WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn vip_is_active(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let now = Utc::now().to_rfc3339();
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM vip_tut_memberships WHERE user_id = ? AND expires_at > ?",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

async fn grant_vip_days(pool: &SqlitePool, user_id: i64, days: i64) -> Result<String> {
    let now = Utc::now();
    let existing = vip_expires_at(pool, user_id).await?;
    let base = existing
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .filter(|value| *value > now)
        .unwrap_or(now);
    let expires_at = (base + Duration::days(days)).to_rfc3339();

    sqlx::query(
        r#"
        INSERT INTO vip_tut_memberships (user_id, expires_at, created_at, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET expires_at = excluded.expires_at, updated_at = excluded.updated_at
        "#,
    )
    .bind(user_id)
    .bind(&expires_at)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(expires_at)
}

async fn list_active_vips(pool: &SqlitePool, limit: i64) -> Result<Vec<(i64, String)>> {
    sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT user_id, expires_at
        FROM vip_tut_memberships
        WHERE expires_at > ?
        ORDER BY expires_at ASC
        LIMIT ?
        "#,
    )
    .bind(Utc::now().to_rfc3339())
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn extend_vip_with_wallet(
    ctx: &AppContext,
    user_id: i64,
    price: i64,
    days: i64,
) -> Result<(String, i64)> {
    let now = Utc::now();
    let existing = vip_expires_at(&ctx.pool, user_id).await?;
    let base = existing
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .filter(|value| *value > now)
        .unwrap_or(now);
    let expires_at = (base + Duration::days(days)).to_rfc3339();
    let order_id = format!("vip-tut-{}-{}", user_id, now.timestamp());

    let mut tx = ctx.pool.begin().await?;
    let balance_after =
        wallet_repo::debit_wallet(&mut tx, user_id, price, &order_id, Some("vip_tut_30_days"))
            .await?;
    sqlx::query(
        r#"
        INSERT INTO vip_tut_memberships (user_id, expires_at, created_at, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET expires_at = excluded.expires_at, updated_at = excluded.updated_at
        "#,
    )
    .bind(user_id)
    .bind(&expires_at)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((expires_at, balance_after))
}

fn parse_start_tut_payload(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;
    if command != "/start" && !command.starts_with("/start@") {
        return None;
    }
    let payload = parts.next()?;
    payload.strip_prefix("tut_")?.parse::<i64>().ok()
}

fn configured_tut_channel(ctx: &AppContext) -> Option<String> {
    let raw = ctx.get_text("vip_tut_channel_id", "");
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_string())
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

fn is_cancel(text: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    first == "/cancel" || first.starts_with("/cancel@")
}

fn parse_tut_content_and_access(text: &str) -> (String, &'static str) {
    let trimmed = text.trim();
    let mut lines = trimmed.lines();
    let Some(first_line) = lines.next() else {
        return (String::new(), "vip");
    };
    let first = first_line.trim().to_lowercase();
    if first == "free" || first == "mien phi" || first == "miễn phí" {
        (lines.collect::<Vec<_>>().join("\n").trim().to_string(), "free")
    } else if first == "vip" {
        (lines.collect::<Vec<_>>().join("\n").trim().to_string(), "vip")
    } else {
        (trimmed.to_string(), "vip")
    }
}

fn tut_is_free(tut: &VipTut) -> bool {
    tut.access_type.eq_ignore_ascii_case("free")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_tut_payload_accepts_bot_deep_link() {
        assert_eq!(parse_start_tut_payload("/start tut_123"), Some(123));
        assert_eq!(parse_start_tut_payload("/start@mmo_hub_bot tut_456"), Some(456));
        assert_eq!(parse_start_tut_payload("/start ref_123"), None);
    }

    #[test]
    fn parse_tut_content_and_access_accepts_free_and_vip() {
        assert_eq!(
            parse_tut_content_and_access("free\nNội dung xem miễn phí"),
            ("Nội dung xem miễn phí".to_string(), "free")
        );
        assert_eq!(
            parse_tut_content_and_access("vip\nNội dung chỉ VIP xem"),
            ("Nội dung chỉ VIP xem".to_string(), "vip")
        );
        assert_eq!(
            parse_tut_content_and_access("Nội dung mặc định là VIP"),
            ("Nội dung mặc định là VIP".to_string(), "vip")
        );
    }

    #[test]
    fn short_label_truncates_long_titles() {
        assert_eq!(short_label("abcdef", 3), "abc…");
        assert_eq!(short_label("abc", 3), "abc");
    }
}
