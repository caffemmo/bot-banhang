use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message};
use url::Url;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, i18n};

pub struct StartAffiliatePlugin;

const AFFILIATE_REGISTER_CALLBACK: &str = "affiliate:register";
const CHILD_BOT_GUIDE_CALLBACK: &str = "childbot:guide";
const DEFAULT_COMMISSION_BPS: i64 = 500;
const ADMIN_CONTACT_URL: &str = "https://t.me/thang_hub";

#[derive(Debug, Clone, FromRow)]
struct StartAffiliatePartner {
    user_id: i64,
    code: String,
    commission_bps: i64,
}

#[async_trait::async_trait]
impl AppPlugin for StartAffiliatePlugin {
    fn name(&self) -> &'static str {
        "CmdStartAffiliate"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_affiliate_partner_schema(pool).await
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_plain_start(text) {
            return Ok(false);
        }

        let lang = if let Some(user) = msg.from() {
            i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            ctx.normalize_language_code(None)
        };
        let msg_text = i18n::t(
            &ctx,
            &lang,
            "start",
            "👋 Welcome! Use the buttons below, or type /shop to buy and /orders to view orders.",
        );
        i18n::send_message_with_json_keyboard(
            &ctx,
            msg.chat.id,
            "start",
            msg_text,
            start_menu_with_affiliate_keyboard_json(&ctx, &lang),
        )
        .await?;
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

        match data {
            AFFILIATE_REGISTER_CALLBACK => {
                let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;
                let _ = ctx
                    .bot
                    .answer_callback_query(q.id.clone())
                    .text(i18n::t(&ctx, &lang, "affiliate_register_ack", "Đang tạo link CTV..."))
                    .await;

                let Some(msg) = &q.message else {
                    return Ok(true);
                };
                register_affiliate_and_send_link(&ctx, msg.chat().id, q.from.id.0 as i64).await?;
                Ok(true)
            }
            CHILD_BOT_GUIDE_CALLBACK => {
                let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
                if let Some(msg) = &q.message {
                    send_child_bot_setup_guide(&ctx, msg.chat().id).await?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn is_plain_start(text: &str) -> bool {
    let mut parts = text.split_whitespace();
    let command = parts.next().unwrap_or("");
    (command == "/start" || command.starts_with("/start@")) && parts.next().is_none()
}

fn start_menu_with_affiliate_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "inline_keyboard": [
            [i18n::inline_button_callback_json(ctx, lang, "start_btn_shop", "🛒 Shop", "start:shop")],
            [
                i18n::inline_button_callback_json(ctx, lang, "start_btn_topup", "💰 Top up", "wallet:topup"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_wallet", "💳 Wallet", "start:wallet"),
            ],
            [
                i18n::inline_button_callback_json(ctx, lang, "start_btn_purchased", "📦 Purchased", "start:orders"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_topup_history", "📜 Top-up history", "wallet:topup_history"),
            ],
            [
                i18n::inline_button_callback_json(ctx, lang, "start_btn_api_integration", "🔌 API integration", "shop_api"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_help", "Help", "start:help"),
            ],
            [i18n::inline_button_callback_json(ctx, lang, "start_btn_affiliate_register", "🤝 Đăng kí CTV", AFFILIATE_REGISTER_CALLBACK)],
            [i18n::inline_button_callback_json(ctx, lang, "start_btn_child_bot", "🤖 Tạo bot con", CHILD_BOT_GUIDE_CALLBACK)],
            [i18n::inline_button_callback_json(ctx, lang, "start_btn_language", "🌐 Language", "start:language")],
        ]
    })
}

async fn register_affiliate_and_send_link(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
) -> Result<()> {
    ensure_affiliate_partner_schema(&ctx.pool).await?;
    let partner = upsert_start_affiliate_partner(&ctx.pool, user_id).await?;
    let url = affiliate_url(ctx, &partner.code).await?;
    let text = format!(
        "✅ Bạn đã đăng kí CTV thành công\n\nLink giới thiệu của bạn:\n{}\n\nCách nhận hoa hồng:\n1. Gửi link này cho bạn bè hoặc khách hàng.\n2. Khi họ bấm link và mua hàng trong bot, bạn nhận {} giá trị đơn.\n3. Hoa hồng được bot ghi nhận tự động.\n\nGõ /ctv để xem thống kê hoa hồng của bạn.",
        url,
        format_percent_bps(partner.commission_bps),
    );

    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "🔗 Mở link CTV",
            Url::parse(&url)?,
        )]]))
        .await?;
    Ok(())
}

async fn send_child_bot_setup_guide(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let text = "🤖 TẠO BOT BÁN HÀNG RIÊNG\n\nBạn có thể có bot bán hàng riêng giống shop chính, chạy trên VPS của bạn.\n\nBạn cần chuẩn bị:\n1. VPS Ubuntu riêng, tối thiểu 1GB RAM để test, khuyên dùng 2GB RAM trở lên.\n2. Token bot Telegram tạo từ @BotFather.\n3. Tên shop hoặc tên bot muốn hiển thị.\n\nQuy trình:\n1. Bạn chuẩn bị VPS và token bot.\n2. Liên hệ admin để được cài bot con.\n3. Bot con sẽ bán hàng bằng dữ liệu từ hệ thống chính.\n4. Đơn hàng và hoa hồng CTV vẫn được ghi nhận tự động.\n\nLưu ý: không gửi token hoặc mật khẩu VPS ở nhóm công khai. Hãy liên hệ trực tiếp admin để được hỗ trợ cài đặt.";

    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "👨‍💻 Liên hệ admin cài đặt",
            Url::parse(ADMIN_CONTACT_URL)?,
        )]]))
        .await?;
    Ok(())
}

async fn ensure_affiliate_partner_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS affiliate_partners (
            user_id INTEGER PRIMARY KEY,
            code TEXT NOT NULL UNIQUE,
            commission_bps INTEGER NOT NULL DEFAULT 500,
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn upsert_start_affiliate_partner(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<StartAffiliatePartner> {
    let code = format!("u{user_id}");
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO affiliate_partners (user_id, code, commission_bps, is_active, created_at, updated_at)
        VALUES (?, ?, ?, 1, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET
            commission_bps = excluded.commission_bps,
            is_active = 1,
            updated_at = excluded.updated_at"#,
    )
    .bind(user_id)
    .bind(&code)
    .bind(DEFAULT_COMMISSION_BPS)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(StartAffiliatePartner {
        user_id,
        code,
        commission_bps: DEFAULT_COMMISSION_BPS,
    })
}

async fn affiliate_url(ctx: &AppContext, code: &str) -> Result<String> {
    let me = ctx.bot.get_me().await?;
    let username = me.user.username.unwrap_or_default();
    Ok(format!("https://t.me/{username}?start=ref_{code}"))
}

fn format_percent_bps(bps: i64) -> String {
    let whole = bps / 100;
    let fraction = bps % 100;
    if fraction == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{fraction:02}%")
    }
}
