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
use crate::bot::plugins::cmd_netflix;
use crate::bot::{BotDialogue, chat_ui, i18n};

pub struct StartAffiliatePlugin;

const JOIN_CHECK_CALLBACK: &str = "start:check_join";
const DEFAULT_REQUIRED_CHANNEL_URL: &str = "https://t.me/zvwboo";
const AFFILIATE_REGISTER_CALLBACK: &str = "affiliate:register";
const DEFAULT_COMMISSION_BPS: i64 = 500;

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
        chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;

        let lang = if let Some(user) = msg.from() {
            i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            ctx.normalize_language_code(None)
        };
        if let Some(user) = msg.from() {
            if !user_has_joined_required_channel(&ctx, user.id).await {
                send_required_channel_prompt(&ctx, msg.chat.id, &lang).await?;
                return Ok(true);
            }
        }
        send_start_menu(&ctx, msg.chat.id, &lang).await?;
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
            JOIN_CHECK_CALLBACK => {
                let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;
                let joined = user_has_joined_required_channel(&ctx, q.from.id).await;
                let ack = if joined {
                    i18n::t(&ctx, &lang, "required_channel_joined", "Đã xác nhận tham gia channel.")
                } else {
                    i18n::t(
                        &ctx,
                        &lang,
                        "required_channel_not_joined",
                        "Bot chưa thấy bạn trong channel, vui lòng tham gia rồi thử lại.",
                    )
                };
                let _ = ctx.bot.answer_callback_query(q.id.clone()).text(ack).await;

                if let Some(msg) = &q.message {
                    if joined {
                        send_start_menu(&ctx, msg.chat().id, &lang).await?;
                    } else {
                        send_required_channel_prompt(&ctx, msg.chat().id, &lang).await?;
                    }
                }
                Ok(true)
            }
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
            _ => Ok(false),
        }
    }
}

fn is_plain_start(text: &str) -> bool {
    let mut parts = text.split_whitespace();
    let command = parts.next().unwrap_or("");
    (command == "/start" || command.starts_with("/start@")) && parts.next().is_none()
}

fn required_channel_enabled_value(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "on" | "yes" | "enabled" | "bat" | "bật")
}

fn normalize_t_me_url(value: &str) -> Option<String> {
    let value = value.trim();
    let stripped = value
        .strip_prefix("https://t.me/")
        .or_else(|| value.strip_prefix("http://t.me/"))
        .or_else(|| value.strip_prefix("t.me/"))?;
    let username = stripped
        .split(['?', '/', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if username.is_empty() || username.starts_with('+') {
        None
    } else if username.starts_with('@') {
        Some(username.to_string())
    } else {
        Some(format!("@{username}"))
    }
}

fn normalize_required_channel_value(value: &str) -> Option<String> {
    let id = value.trim();
    if id.is_empty() {
        return None;
    }

    if id.starts_with("http://t.me/") || id.starts_with("https://t.me/") || id.starts_with("t.me/")
    {
        return normalize_t_me_url(id);
    }

    if id.starts_with('@') || id.starts_with("-100") {
        Some(id.to_string())
    } else {
        Some(format!("@{id}"))
    }
}

fn push_required_channel_candidate(candidates: &mut Vec<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    if !candidates.iter().any(|value| value == &candidate) {
        candidates.push(candidate);
    }
}

fn required_channel_candidates(channel_id: &str, channel_url: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    push_required_channel_candidate(&mut candidates, normalize_t_me_url(channel_url));
    push_required_channel_candidate(&mut candidates, normalize_required_channel_value(channel_id));
    candidates
}

fn required_channel_enabled(ctx: &AppContext) -> bool {
    required_channel_enabled_value(&ctx.get_text("required_channel_enabled", "1"))
}

fn start_viameta_enabled(ctx: &AppContext) -> bool {
    required_channel_enabled_value(&ctx.get_text("start_viameta_enabled", "0"))
}

fn required_channel_ids(ctx: &AppContext) -> Vec<String> {
    required_channel_candidates(
        &ctx.get_text("required_channel_id", "@zvwboo"),
        &ctx.get_text("required_channel_url", DEFAULT_REQUIRED_CHANNEL_URL),
    )
}

fn required_channel_url(ctx: &AppContext) -> String {
    let url = ctx
        .get_text("required_channel_url", DEFAULT_REQUIRED_CHANNEL_URL)
        .trim()
        .to_string();
    if url.is_empty() {
        DEFAULT_REQUIRED_CHANNEL_URL.to_string()
    } else {
        url
    }
}

async fn user_has_joined_required_channel(ctx: &AppContext, user_id: teloxide::types::UserId) -> bool {
    if !required_channel_enabled(ctx) {
        return true;
    }

    let channel_ids = required_channel_ids(ctx);
    if channel_ids.is_empty() {
        return true;
    }

    for channel_id in channel_ids {
        match ctx.bot.get_chat_member(channel_id.clone(), user_id).await {
            Ok(member) if member.kind.is_present() => return true,
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    "Failed to check required channel membership for {channel_id}: {err}"
                );
            }
        }
    }
    false
}

fn join_required_channel_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    let channel_url = required_channel_url(ctx);
    let join_text = i18n::button_t(ctx, lang, "required_channel_join_btn", "📢 Tham gia channel");
    let check_button = i18n::inline_button_callback_json(
        ctx,
        lang,
        "required_channel_check_btn",
        "✅ Tôi đã tham gia",
        JOIN_CHECK_CALLBACK,
    );

    let mut rows = Vec::new();
    if let Ok(url) = Url::parse(&channel_url) {
        rows.push(vec![json!({ "text": join_text, "url": url.as_str() })]);
    }
    rows.push(vec![check_button]);
    json!({ "inline_keyboard": rows })
}

async fn send_required_channel_prompt(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let channel_url = required_channel_url(ctx);
    let text = ctx
        .render_text_lang(
            "required_channel_message",
            lang,
            "📢 Vui lòng tham gia channel trước khi sử dụng bot:\n{channel_url}\n\nSau khi tham gia, bấm “Tôi đã tham gia”.",
            &[("channel_url", channel_url.clone())],
        );
    chat_ui::send_clean_menu(
        ctx,
        chat_id,
        "required_channel_message",
        text,
        join_required_channel_keyboard_json(ctx, lang),
    )
    .await?;
    Ok(())
}

async fn send_start_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let msg_text = i18n::t(
        ctx,
        lang,
        "start",
        "👋 Welcome! Use the buttons below, or type /shop to buy and /orders to view orders.",
    );
    chat_ui::send_clean_menu(
        ctx,
        chat_id,
        "start",
        msg_text,
        start_menu_with_affiliate_keyboard_json(ctx, lang),
    )
    .await?;
    Ok(())
}

fn start_menu_with_affiliate_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    let mut shop_row = vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "start_btn_shop",
        "🛒 Shop",
        "start:shop",
    )];
    if cmd_netflix::netflix_enabled(ctx) {
        shop_row.push(cmd_netflix::netflix_button_json(ctx, lang));
    }

    let mut rows = vec![
        shop_row,
        vec![
            i18n::inline_button_callback_json(
                ctx,
                lang,
                "start_btn_topup",
                "💰 Top up",
                "wallet:topup",
            ),
            i18n::inline_button_callback_json(
                ctx,
                lang,
                "start_btn_wallet",
                "💳 Wallet",
                "start:wallet",
            ),
        ],
        vec![
            i18n::inline_button_callback_json(
                ctx,
                lang,
                "start_btn_purchased",
                "📦 Purchased",
                "start:orders",
            ),
            i18n::inline_button_callback_json(
                ctx,
                lang,
                "start_btn_topup_history",
                "📜 Top-up history",
                "wallet:topup_history",
            ),
        ],
    ];

    let mut support_row = vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "start_btn_help",
        "Help",
        "start:help",
    )];
    if start_viameta_enabled(ctx) {
        support_row.push(i18n::inline_button_callback_json(
            ctx,
            lang,
            "start_btn_viameta",
            "✅ Up tích xanh",
            "viameta:menu",
        ));
    }
    rows.push(support_row);
    rows.push(vec![
        i18n::inline_button_callback_json(
            ctx,
            lang,
            "start_btn_affiliate_register",
            "🤝 Đăng kí CTV",
            AFFILIATE_REGISTER_CALLBACK,
        ),
        start_community_button_json(ctx, lang),
    ]);
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "start_btn_language",
        "🌐 Language",
        "start:language",
    )]);

    json!({ "inline_keyboard": rows })
}

fn start_community_button_json(ctx: &AppContext, lang: &str) -> Value {
    let channel_url = required_channel_url(ctx);
    if Url::parse(&channel_url).is_ok() {
        inline_button_url_json(
            ctx,
            "start_btn_community",
            i18n::t(ctx, lang, "start_btn_community", "👥 Community"),
            channel_url,
        )
    } else {
        i18n::inline_button_callback_json(
            ctx,
            lang,
            "start_btn_community",
            "👥 Community",
            "start:help",
        )
    }
}

fn inline_button_url_json(
    ctx: &AppContext,
    key: &str,
    text: impl Into<String>,
    url: impl Into<String>,
) -> Value {
    let parts = i18n::button_parts_for_key(ctx, key, text);
    let mut button = json!({
        "text": parts.text,
        "url": url.into(),
    });
    if let Some(icon_id) = parts.icon_custom_emoji_id
        && let Some(obj) = button.as_object_mut()
    {
        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
    }
    button
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
