use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use rand::{Rng, distributions::Alphanumeric};
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, PRAGMA, REFERER, USER_AGENT,
};
use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use teloxide::payloads::{SendDocumentSetters, SendMessageSetters, SendVideoSetters};
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile,
    Message, ParseMode,
};
use url::Url;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::{BotDialogue, i18n};
use crate::domains::orders::api::html_escape;
use crate::domains::wallet::repo as wallet_repo;

const GET_COOKIE_URL_DEFAULT: &str = "https://api.tiembanh4k.com/api/ctv-api/get-cookie";
const REGENERATE_URL_DEFAULT: &str =
    "https://backend-c0r3-7xpq9zn2025.onrender.com/api/ctv-api/regenerate-token";
const API_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0";

pub struct NetflixCommandPlugin;

#[derive(Debug, Clone)]
struct NetflixCookie {
    log_id: String,
    cookie_number: Option<i64>,
    cookie: Option<String>,
    mobile_login_link: Option<String>,
    pc_login_link: Option<String>,
    token_expires: Option<i64>,
    time_remaining: Option<i64>,
}

#[derive(Debug, Clone)]
struct NetflixSession {
    id: i64,
    log_id: String,
    cookie_number: Option<i64>,
    cookie: Option<String>,
}

#[async_trait::async_trait]
impl AppPlugin for NetflixCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdNetflix"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_netflix_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "netflix".to_string(),
            description: "Xem Netflix".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if text == "/netflix"
            || text.eq_ignore_ascii_case("🎬 Xem Netflix")
            || text.eq_ignore_ascii_case("Xem Netflix")
        {
            let lang = if let Some(user) = msg.from() {
                i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
            } else {
                ctx.normalize_language_code(None)
            };
            send_netflix_menu(&ctx, msg.chat.id, &lang).await?;
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
        if !data.starts_with("netflix:") {
            return Ok(false);
        }

        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };
        let chat_id = msg.chat().id;
        let user_id = q.from.id.0 as i64;
        let lang = i18n::user_lang(&ctx, user_id, q.from.language_code.as_deref()).await;

        if data == "netflix:menu" {
            send_netflix_menu(&ctx, chat_id, &lang).await?;
        } else if data == "netflix:buy" {
            handle_netflix_buy(&ctx, chat_id, user_id, &lang).await?;
        } else if data == "netflix:pc_guide" {
            send_netflix_pc_guide(&ctx, chat_id).await?;
        } else if data == "netflix:language_vi_guide" {
            send_netflix_language_vi_guide(&ctx, chat_id).await?;
        } else if data == "netflix:mobile_guide" {
            send_netflix_mobile_guide(&ctx, chat_id).await?;
        } else if data == "netflix:mobile_language_guide" {
            send_netflix_mobile_language_guide(&ctx, chat_id).await?;
        } else if let Some(id) = data
            .strip_prefix("netflix:cookie:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            handle_netflix_cookie(&ctx, chat_id, user_id, id).await?;
        } else if let Some(id) = data
            .strip_prefix("netflix:regen:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            handle_netflix_regen(&ctx, chat_id, user_id, id, &lang).await?;
        }

        Ok(true)
    }
}

pub fn netflix_enabled(ctx: &AppContext) -> bool {
    config_bool(ctx, "netflix_enabled", true)
}

pub fn netflix_button_json(ctx: &AppContext, lang: &str) -> Value {
    i18n::inline_button_callback_json(
        ctx,
        lang,
        "start_btn_netflix",
        "🎬 Xem Netflix",
        "netflix:menu",
    )
}

async fn send_netflix_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    if !netflix_enabled(ctx) {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_disabled_message",
                    "🎬 Netflix hiện đang tắt, vui lòng quay lại sau.",
                ),
            )
            .await?;
        return Ok(());
    }

    let price = netflix_price(ctx);
    let wallet_hint = if price > 0 {
        format!(
            "\n{}: <b>{}</b>",
            netflix_text(ctx, "netflix_price_label", "Giá"),
            format_vnd(price)
        )
    } else {
        format!(
            "\n{}: <b>{}</b>",
            netflix_text(ctx, "netflix_price_label", "Giá"),
            netflix_text(ctx, "netflix_free_label", "Miễn phí")
        )
    };
    let text = format!(
        "{}\n\n{}{wallet_hint}\n\n{}",
        netflix_text(ctx, "netflix_menu_title", "🎬 <b>XEM NETFLIX</b>"),
        netflix_text(
            ctx,
            "netflix_menu_description",
            "Bấm nút bên dưới để lấy cookie và link đăng nhập Netflix."
        ),
        netflix_text(
            ctx,
            "netflix_menu_note",
            "Link đăng nhập có hạn khoảng 1 giờ. Khi hết hạn, bấm <b>Tạo lại link</b>."
        )
    );

    let buy_button_text = netflix_text(ctx, "netflix_buy_button_text", "🎬 Lấy Netflix");
    let mut menu_rows = vec![vec![InlineKeyboardButton::callback(
        if price > 0 {
            format!("{} ({})", buy_button_text, format_vnd(price))
        } else {
            buy_button_text
        },
        "netflix:buy",
    )]];
    if let Some(button) = netflix_pc_guide_button(ctx) {
        menu_rows.push(vec![button]);
    }
    if let Some(button) = netflix_language_vi_guide_button(ctx) {
        menu_rows.push(vec![button]);
    }
    if let Some(button) = netflix_mobile_guide_button(ctx) {
        menu_rows.push(vec![button]);
    }
    if let Some(button) = netflix_mobile_language_guide_button(ctx) {
        menu_rows.push(vec![button]);
    }
    menu_rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Ví tiền",
        "start:wallet",
    )]);
    menu_rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại",
        "start:menu",
    )]);

    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(menu_rows))
        .await?;

    Ok(())
}

async fn handle_netflix_buy(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    if !netflix_enabled(ctx) {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_disabled_message",
                    "🎬 Netflix hiện đang tắt, vui lòng quay lại sau.",
                ),
            )
            .await?;
        return Ok(());
    }

    let Some(api_key) = netflix_api_key(ctx) else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(ctx, "netflix_get_error_message", "⚠️ Get lỗi, vui lòng thử lại sau."),
            )
            .await?;
        return Ok(());
    };
    let price = netflix_price(ctx);
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;
    if price > 0 && wallet.balance < price {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "⚠️ Số dư ví không đủ.\nSố dư hiện tại: {}\nCần: {}",
                    format_vnd(wallet.balance),
                    format_vnd(price)
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![
                vec![i18n::inline_button_callback(
                    ctx,
                    lang,
                    "start_btn_topup",
                    "💰 Nạp tiền",
                    "wallet:topup",
                )],
                vec![InlineKeyboardButton::callback("⬅️ Quay lại", "netflix:menu")],
            ]))
            .await?;
        return Ok(());
    }

    let order_id = netflix_order_id();
    if price > 0 {
        let mut tx = ctx.pool.begin().await?;
        wallet_repo::debit_wallet(
            &mut tx,
            user_id,
            price,
            &order_id,
            Some("netflix_cookie_purchase"),
        )
        .await?;
        tx.commit().await?;
    }

    ctx.bot
        .send_message(
            chat_id,
            netflix_text(
                ctx,
                "netflix_loading_message",
                "⏳ Đang lấy Netflix, vui lòng chờ...",
            ),
        )
        .await?;

    match call_get_cookie_api(ctx, &api_key).await {
        Ok(cookie) => {
            let session_id = save_netflix_session(ctx, user_id, chat_id.0, &cookie).await?;
            send_netflix_cookie(ctx, chat_id, session_id, &cookie, price).await?;
        }
        Err(err) => {
            if price > 0 {
                refund_netflix_purchase(ctx, user_id, price, &order_id, &err.to_string()).await?;
            }
            ctx.bot
                .send_message(
                    chat_id,
                    html_escape(&netflix_text(
                        ctx,
                        "netflix_get_error_message",
                        "⚠️ Get lỗi, vui lòng thử lại sau.",
                    )),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback(
                        netflix_text(ctx, "netflix_retry_button_text", "🔄 Thử lại"),
                        "netflix:buy",
                    ),
                    InlineKeyboardButton::callback("⬅️ Quay lại", "netflix:menu"),
                ]]))
                .await?;
        }
    }

    Ok(())
}

async fn handle_netflix_regen(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    session_id: i64,
    _lang: &str,
) -> Result<()> {
    let Some(api_key) = netflix_api_key(ctx) else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_regen_error_message",
                    "⚠️ Tạo lại link lỗi, vui lòng thử lại sau.",
                ),
            )
            .await?;
        return Ok(());
    };
    let Some(session) = get_netflix_session(ctx, session_id, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_session_missing_message",
                    "Không tìm thấy phiên Netflix này.",
                ),
            )
            .await?;
        return Ok(());
    };

    ctx.bot
        .send_message(
            chat_id,
            netflix_text(
                ctx,
                "netflix_regen_loading_message",
                "⏳ Đang tạo lại link Netflix...",
            ),
        )
        .await?;

    match call_regen_api(ctx, &api_key, &session.log_id).await {
        Ok((pc_link, mobile_link, expires_at)) => {
            update_netflix_session_links(ctx, session_id, &pc_link, &mobile_link, expires_at).await?;
            let cookie = NetflixCookie {
                log_id: session.log_id,
                cookie_number: session.cookie_number,
                cookie: session.cookie,
                mobile_login_link: Some(mobile_link),
                pc_login_link: Some(pc_link),
                token_expires: expires_at,
                time_remaining: None,
            };
            send_netflix_cookie(ctx, chat_id, session.id, &cookie, 0).await?;
        }
        Err(_err) => {
            ctx.bot
                .send_message(
                    chat_id,
                    html_escape(&netflix_text(
                        ctx,
                        "netflix_regen_error_message",
                        "⚠️ Tạo lại link lỗi, vui lòng thử lại sau.",
                    )),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback(
                        netflix_text(ctx, "netflix_retry_button_text", "🔄 Thử lại"),
                        format!("netflix:regen:{session_id}"),
                    ),
                    InlineKeyboardButton::callback("⬅️ Menu Netflix", "netflix:menu"),
                ]]))
                .await?;
        }
    }

    Ok(())
}

async fn send_netflix_cookie(
    ctx: &AppContext,
    chat_id: ChatId,
    session_id: i64,
    cookie: &NetflixCookie,
    price: i64,
) -> Result<()> {
    let mut rows = Vec::new();
    let mut link_row = Vec::new();
    if let Some(link) = cookie.pc_login_link.as_deref().and_then(url_button_link) {
        link_row.push(InlineKeyboardButton::url(
            netflix_text(ctx, "netflix_pc_button_text", "💻 Mở PC"),
            link,
        ));
    }
    if let Some(link) = cookie.mobile_login_link.as_deref().and_then(url_button_link) {
        link_row.push(InlineKeyboardButton::url(
            netflix_text(ctx, "netflix_mobile_button_text", "📱 Mở Mobile"),
            link,
        ));
    }
    if !link_row.is_empty() {
        rows.push(link_row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        netflix_text(ctx, "netflix_cookie_button_text", "🍪 Lấy cookie"),
        format!("netflix:cookie:{session_id}"),
    )]);
    if let Some(button) = netflix_pc_guide_button(ctx) {
        rows.push(vec![button]);
    }
    if let Some(button) = netflix_language_vi_guide_button(ctx) {
        rows.push(vec![button]);
    }
    if let Some(button) = netflix_mobile_guide_button(ctx) {
        rows.push(vec![button]);
    }
    if let Some(button) = netflix_mobile_language_guide_button(ctx) {
        rows.push(vec![button]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        netflix_text(ctx, "netflix_regen_button_text", "🔄 Tạo lại link"),
        format!("netflix:regen:{session_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        netflix_text(ctx, "netflix_buy_again_button_text", "🎬 Lấy Netflix khác"),
        "netflix:buy",
    )]);

    let mut text = format!(
        "{}\n\n{}: <code>{}</code>",
        netflix_text(ctx, "netflix_success_title", "✅ <b>NETFLIX ĐÃ SẴN SÀNG</b>"),
        netflix_text(ctx, "netflix_account_code_label", "Mã tài khoản"),
        cookie
            .cookie_number
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    if price > 0 {
        text.push_str(&format!(
            "\n{}: <b>{}</b>",
            netflix_text(ctx, "netflix_wallet_deducted_label", "Đã trừ ví"),
            format_vnd(price)
        ));
    }
    if let Some(time) = cookie.time_remaining {
        text.push_str(&format!(
            "\n{}: <b>{}</b>",
            netflix_text(ctx, "netflix_time_remaining_label", "Link còn hạn khoảng"),
            format_seconds(time)
        ));
    }
    text.push_str(&format!(
        "\n\n{}",
        netflix_text(
            ctx,
            "netflix_success_note",
            "Bấm nút bên dưới để mở Netflix. Nếu link hết hạn, bấm Tạo lại link."
        )
    ));

    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;

    Ok(())
}

async fn handle_netflix_cookie(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    session_id: i64,
) -> Result<()> {
    let Some(session) = get_netflix_session(ctx, session_id, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_session_missing_message",
                    "Không tìm thấy phiên Netflix này.",
                ),
            )
            .await?;
        return Ok(());
    };
    let Some(raw_cookie) = session.cookie.as_deref().filter(|value| !value.trim().is_empty())
    else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_cookie_missing_message",
                    "⚠️ Chưa có cookie cho lượt này.",
                ),
            )
            .await?;
        return Ok(());
    };

    send_netflix_cookie_value(ctx, chat_id, session_id, raw_cookie).await
}

async fn send_netflix_cookie_value(
    ctx: &AppContext,
    chat_id: ChatId,
    session_id: i64,
    raw_cookie: &str,
) -> Result<()> {
    if raw_cookie.chars().count() <= 3300 {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "{}\n<pre>{}</pre>",
                    netflix_text(ctx, "netflix_cookie_title", "🍪 <b>Cookie Netflix</b>"),
                    html_escape(raw_cookie)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
    } else {
        ctx.bot
            .send_document(
                chat_id,
                InputFile::memory(raw_cookie.as_bytes().to_vec())
                    .file_name(format!("netflix_cookie_{session_id}.txt")),
            )
            .caption(netflix_text(
                ctx,
                "netflix_cookie_file_caption",
                "🍪 Cookie Netflix được gửi trong file.",
            ))
            .await?;
    }
    Ok(())
}

async fn send_netflix_mobile_language_guide(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    send_netflix_guide_video(
        ctx,
        chat_id,
        "netflix_mobile_language_guide_video_path",
        "public/assets/netflix/mobile-language-guide.mp4",
        "netflix_mobile_language_guide_caption",
        "🌐 Cách đổi ngôn ngữ Mobile",
        "netflix_mobile_language_guide_missing_message",
        "⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.",
    )
    .await
}

async fn send_netflix_mobile_guide(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    send_netflix_guide_video(
        ctx,
        chat_id,
        "netflix_mobile_guide_video_path",
        "public/assets/netflix/mobile-guide.mov",
        "netflix_mobile_guide_caption",
        "📱 Cách coi trên Mobie",
        "netflix_mobile_guide_missing_message",
        "⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.",
    )
    .await
}

async fn send_netflix_language_vi_guide(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    send_netflix_guide_video(
        ctx,
        chat_id,
        "netflix_language_vi_guide_video_path",
        "public/assets/netflix/language-vi-guide.mp4",
        "netflix_language_vi_guide_caption",
        "🌐 Hướng dẫn đổi ngôn ngữ sang Tiếng Việt",
        "netflix_language_vi_guide_missing_message",
        "⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.",
    )
    .await
}

async fn send_netflix_pc_guide(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    send_netflix_guide_video(
        ctx,
        chat_id,
        "netflix_pc_guide_video_path",
        "public/assets/netflix/pc-guide.mp4",
        "netflix_pc_guide_caption",
        "💻 Hướng dẫn xem Netflix trên PC",
        "netflix_pc_guide_missing_message",
        "⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.",
    )
    .await
}

async fn send_netflix_guide_video(
    ctx: &AppContext,
    chat_id: ChatId,
    path_key: &str,
    default_path: &str,
    caption_key: &str,
    default_caption: &str,
    missing_key: &str,
    default_missing: &str,
) -> Result<()> {
    let path = netflix_text(
        ctx,
        path_key,
        default_path,
    );
    let Some(video) = guide_video_input(&path) else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    missing_key,
                    default_missing,
                ),
            )
            .await?;
        return Ok(());
    };

    let caption = netflix_text(ctx, caption_key, default_caption);
    let send_video_result = ctx.bot
        .send_video(chat_id, video)
        .caption(caption.clone())
        .supports_streaming(true)
        .await;
    if send_video_result.is_err()
        && let Some(document) = guide_video_input(&path)
    {
        ctx.bot.send_document(chat_id, document).caption(caption).await?;
    } else {
        send_video_result?;
    }

    Ok(())
}

async fn call_get_cookie_api(ctx: &AppContext, api_key: &str) -> Result<NetflixCookie> {
    let url = api_url_with_key(
        &ctx.get_text("netflix_get_cookie_url", GET_COOKIE_URL_DEFAULT),
        api_key,
    )?;
    let response = netflix_client(ctx)?
        .get(url)
        .netflix_api_headers(api_key)
        .send()
        .await?;
    let value = read_api_json(response).await?;

    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(api_error_message(&value)));
    }

    let log_id = json_string(&value, "logId")
        .ok_or_else(|| anyhow!("API không trả về logId"))?
        .to_string();
    let mobile = json_string(&value, "mobileLoginLink").map(ToString::to_string);
    let pc = json_string(&value, "pcLoginLink")
        .map(ToString::to_string)
        .or_else(|| mobile.as_ref().map(|link| mobile_to_pc_link(link)));

    Ok(NetflixCookie {
        log_id,
        cookie_number: json_i64(&value, "cookieNumber"),
        cookie: json_string(&value, "cookie").map(ToString::to_string),
        mobile_login_link: mobile,
        pc_login_link: pc,
        token_expires: json_i64(&value, "tokenExpires"),
        time_remaining: json_i64(&value, "timeRemaining"),
    })
}

async fn call_regen_api(
    ctx: &AppContext,
    api_key: &str,
    log_id: &str,
) -> Result<(String, String, Option<i64>)> {
    let url = api_url_with_key(
        &ctx.get_text("netflix_regenerate_url", REGENERATE_URL_DEFAULT),
        api_key,
    )?;
    let response = netflix_client(ctx)?
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .netflix_api_headers(api_key)
        .json(&json!({ "logId": log_id }))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    let value = read_api_json(response).await?;

    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(api_error_message(&value)));
    }

    let mobile = json_string(&value, "tokenLink")
        .or_else(|| json_string(&value, "mobileLoginLink"))
        .ok_or_else(|| anyhow!("API không trả về tokenLink"))?
        .to_string();
    let pc = json_string(&value, "pcLoginLink")
        .map(ToString::to_string)
        .unwrap_or_else(|| mobile_to_pc_link(&mobile));
    Ok((pc, mobile, json_i64(&value, "tokenExpires")))
}

async fn ensure_netflix_schema(pool: &crate::db::DbPool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS netflix_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            log_id TEXT NOT NULL,
            cookie_number INTEGER,
            cookie TEXT,
            pc_login_link TEXT,
            mobile_login_link TEXT,
            token_expires INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_netflix_sessions_user_chat ON netflix_sessions(user_id, chat_id, id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn save_netflix_session(
    ctx: &AppContext,
    user_id: i64,
    chat_id: i64,
    cookie: &NetflixCookie,
) -> Result<i64> {
    let id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO netflix_sessions
        (user_id, chat_id, log_id, cookie_number, cookie, pc_login_link, mobile_login_link, token_expires, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
        RETURNING id"#,
    )
    .bind(user_id)
    .bind(chat_id)
    .bind(&cookie.log_id)
    .bind(cookie.cookie_number)
    .bind(&cookie.cookie)
    .bind(&cookie.pc_login_link)
    .bind(&cookie.mobile_login_link)
    .bind(cookie.token_expires)
    .fetch_one(&ctx.pool)
    .await?;
    Ok(id)
}

async fn get_netflix_session(
    ctx: &AppContext,
    session_id: i64,
    user_id: i64,
    chat_id: i64,
) -> Result<Option<NetflixSession>> {
    let row = sqlx::query_as::<_, (i64, String, Option<i64>, Option<String>)>(
        r#"SELECT id, log_id, cookie_number, cookie
        FROM netflix_sessions
        WHERE id = ? AND user_id = ? AND chat_id = ?"#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind(chat_id)
    .fetch_optional(&ctx.pool)
    .await?;

    Ok(row.map(|(id, log_id, cookie_number, cookie)| NetflixSession {
        id,
        log_id,
        cookie_number,
        cookie,
    }))
}

async fn update_netflix_session_links(
    ctx: &AppContext,
    session_id: i64,
    pc_link: &str,
    mobile_link: &str,
    token_expires: Option<i64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE netflix_sessions
        SET pc_login_link = ?, mobile_login_link = ?, token_expires = ?, updated_at = datetime('now')
        WHERE id = ?"#,
    )
    .bind(pc_link)
    .bind(mobile_link)
    .bind(token_expires)
    .bind(session_id)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

async fn refund_netflix_purchase(
    ctx: &AppContext,
    user_id: i64,
    amount: i64,
    order_id: &str,
    _reason: &str,
) -> Result<()> {
    let mut tx = ctx.pool.begin().await?;
    let note = "Hoàn tiền Netflix do get lỗi";
    wallet_repo::credit_wallet(
        &mut tx,
        user_id,
        amount,
        "refund",
        Some(order_id),
        None,
        Some(note),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn netflix_api_key(ctx: &AppContext) -> Option<String> {
    ctx.get_text("netflix_ctv_api_key", "")
        .trim()
        .to_string()
        .into_nonempty()
}

fn netflix_price(ctx: &AppContext) -> i64 {
    ctx.get_text("netflix_price", "0")
        .trim()
        .parse::<i64>()
        .unwrap_or(0)
        .max(0)
}

fn netflix_pc_guide_button(ctx: &AppContext) -> Option<InlineKeyboardButton> {
    if !config_bool(ctx, "netflix_pc_guide_enabled", true) {
        return None;
    }
    Some(InlineKeyboardButton::callback(
        netflix_text(
            ctx,
            "netflix_pc_guide_button_text",
            "💻 Hướng dẫn xem trên PC",
        ),
        "netflix:pc_guide",
    ))
}

fn netflix_language_vi_guide_button(ctx: &AppContext) -> Option<InlineKeyboardButton> {
    if !config_bool(ctx, "netflix_language_vi_guide_enabled", true) {
        return None;
    }
    Some(InlineKeyboardButton::callback(
        netflix_text(
            ctx,
            "netflix_language_vi_guide_button_text",
            "🌐 Hướng dẫn đổi ngôn ngữ sang Tiếng Việt",
        ),
        "netflix:language_vi_guide",
    ))
}

fn netflix_mobile_guide_button(ctx: &AppContext) -> Option<InlineKeyboardButton> {
    if !config_bool(ctx, "netflix_mobile_guide_enabled", true) {
        return None;
    }
    Some(InlineKeyboardButton::callback(
        netflix_text(
            ctx,
            "netflix_mobile_guide_button_text",
            "📱 Cách coi trên Mobie",
        ),
        "netflix:mobile_guide",
    ))
}

fn netflix_mobile_language_guide_button(ctx: &AppContext) -> Option<InlineKeyboardButton> {
    if !config_bool(ctx, "netflix_mobile_language_guide_enabled", true) {
        return None;
    }
    Some(InlineKeyboardButton::callback(
        netflix_text(
            ctx,
            "netflix_mobile_language_guide_button_text",
            "🌐 Cách đổi ngôn ngữ Mobile",
        ),
        "netflix:mobile_language_guide",
    ))
}

fn netflix_text(ctx: &AppContext, key: &str, default: &str) -> String {
    let value = ctx.get_text(key, default);
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn netflix_client(ctx: &AppContext) -> Result<Client> {
    let mut builder = Client::builder();
    if let Some(proxy_url) = ctx
        .get_text("netflix_proxy_url", "")
        .trim()
        .to_string()
        .into_nonempty()
    {
        builder = builder.proxy(
            reqwest::Proxy::all(&proxy_url).context("Proxy Netflix không hợp lệ")?,
        );
    }
    builder.build().context("Không tạo được HTTP client Netflix")
}

fn config_bool(ctx: &AppContext, key: &str, default: bool) -> bool {
    let default_value = if default { "1" } else { "0" };
    matches!(
        ctx.get_text(key, default_value)
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "on" | "yes" | "enabled" | "enable" | "bat" | "bật"
    )
}

fn netflix_order_id() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect();
    format!("NFLX{}{}", chrono::Utc::now().timestamp(), suffix)
}

fn url_button_link(value: &str) -> Option<Url> {
    Url::parse(value).ok()
}

fn guide_video_input(value: &str) -> Option<InputFile> {
    let value = value.trim();
    if let Ok(url) = Url::parse(value) {
        return Some(InputFile::url(url));
    }
    if Path::new(value).exists() {
        return Some(InputFile::file(value.to_string()));
    }
    None
}

fn api_url_with_key(raw_url: &str, api_key: &str) -> Result<String> {
    let mut url = Url::parse(raw_url.trim())?;
    url.query_pairs_mut().append_pair("apikey", api_key);
    Ok(url.to_string())
}

fn mobile_to_pc_link(value: &str) -> String {
    value.replace("unsupported", "browse")
}

fn json_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn json_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|raw| {
        raw.as_i64()
            .or_else(|| raw.as_u64().and_then(|v| i64::try_from(v).ok()))
            .or_else(|| raw.as_str().and_then(|v| v.parse::<i64>().ok()))
    })
}

fn api_error_message(value: &Value) -> String {
    json_string(value, "message")
        .or_else(|| json_string(value, "error"))
        .unwrap_or("API trả về thất bại")
        .to_string()
}

trait NetflixApiRequestHeaders {
    fn netflix_api_headers(self, api_key: &str) -> Self;
}

impl NetflixApiRequestHeaders for RequestBuilder {
    fn netflix_api_headers(self, api_key: &str) -> Self {
        self.header("X-API-Key", api_key)
            .header(ACCEPT, "application/json, text/plain, */*")
            .header(ACCEPT_LANGUAGE, "vi-VN,vi;q=0.9,en-US;q=0.8,en;q=0.7")
            .header(CACHE_CONTROL, "no-cache")
            .header(PRAGMA, "no-cache")
            .header(REFERER, "https://api.tiembanh4k.com/")
            .header("Origin", "https://api.tiembanh4k.com")
            .header("sec-fetch-site", "none")
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-dest", "document")
            .header("upgrade-insecure-requests", "1")
            .header(USER_AGENT, API_USER_AGENT)
    }
}

async fn read_api_json(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        if status.as_u16() == 403 && looks_like_html(&body) {
            return Err(anyhow!(
                "API Netflix trả HTTP 403: máy chủ bên thứ 3 đang chặn request từ bot/VPS"
            ));
        }
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .map(|value| api_error_message(&value))
            .unwrap_or_else(|| friendly_error(&body));
        return Err(anyhow!("API Netflix trả HTTP {}: {}", status.as_u16(), detail));
    }
    serde_json::from_str::<Value>(&body).context("API Netflix trả dữ liệu không phải JSON")
}

fn looks_like_html(value: &str) -> bool {
    let value = value.trim_start().to_ascii_lowercase();
    value.starts_with("<!doctype html") || value.starts_with("<html")
}

fn friendly_error(value: &str) -> String {
    value.lines().next().unwrap_or(value).chars().take(180).collect()
}

fn format_seconds(seconds: i64) -> String {
    if seconds <= 0 {
        return "hết hạn".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        format!("{minutes} phút")
    } else {
        format!("{} giờ {} phút", minutes / 60, minutes % 60)
    }
}

trait NonEmptyString {
    fn into_nonempty(self) -> Option<String>;
}

impl NonEmptyString for String {
    fn into_nonempty(self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_to_pc_link_replaces_unsupported_path() {
        assert_eq!(
            mobile_to_pc_link("https://netflix.com/unsupported?token=abc"),
            "https://netflix.com/browse?token=abc"
        );
    }

    #[test]
    fn json_i64_reads_number_or_string() {
        assert_eq!(json_i64(&json!({"v": 42}), "v"), Some(42));
        assert_eq!(json_i64(&json!({"v": "42"}), "v"), Some(42));
    }

    #[test]
    fn api_url_with_key_appends_query_param() {
        assert_eq!(
            api_url_with_key("https://api.example.test/get-cookie?foo=1", "ctv_secret").unwrap(),
            "https://api.example.test/get-cookie?foo=1&apikey=ctv_secret"
        );
    }

    #[test]
    fn config_bool_accepts_admin_toggle_values() {
        let config = crate::config::Config::from_env_map(&std::collections::HashMap::from([
            ("TELEGRAM_TOKEN".to_string(), "test".to_string()),
            ("DATABASE_URL".to_string(), "sqlite::memory:".to_string()),
            ("WEBHOOK_SECRET".to_string(), "secret".to_string()),
            ("ADMIN_JWT_SECRET".to_string(), "12345678901234567890123456789012".to_string()),
            ("ADMIN_SETUP_CODE".to_string(), "setup".to_string()),
        ]))
        .unwrap();
        let ctx = AppContext::new(
            Bot::new("test"),
            sqlx::sqlite::SqlitePoolOptions::new()
                .connect_lazy("sqlite::memory:")
                .unwrap(),
            config,
            std::collections::HashMap::from([("netflix_enabled".to_string(), "bật".to_string())]),
            crate::bot::texts::BotTexts::default(),
            vec![],
        );

        assert!(netflix_enabled(&ctx));
    }
}
