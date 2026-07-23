use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::{Datelike, TimeZone, Utc};
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
const MONTHLY_GIFT_START_AT_DEFAULT: &str = "2026-07-23T00:00:00+07:00";
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
    user_id: i64,
    chat_id: i64,
    log_id: String,
    cookie_number: Option<i64>,
    cookie: Option<String>,
    pc_login_link: Option<String>,
    mobile_login_link: Option<String>,
    purchase_amount: Option<i64>,
}

#[derive(Debug, Clone)]
struct NetflixCookieReport {
    id: i64,
    session: NetflixSession,
    status: String,
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
        if !data.starts_with("netflix:") && !data.starts_with("netflixreport:") {
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
        } else if data == "netflix:reopen_latest" {
            handle_netflix_reopen_latest(&ctx, chat_id, user_id, &lang).await?;
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
        } else if let Some(id) = data
            .strip_prefix("netflix:report:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            handle_netflix_cookie_report(&ctx, chat_id, user_id, id).await?;
        } else if let Some(id) = data
            .strip_prefix("netflixreport:refund:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            handle_netflix_report_refund(&ctx, chat_id, user_id, id).await?;
        } else if let Some(id) = data
            .strip_prefix("netflixreport:ok:")
            .and_then(|raw| raw.parse::<i64>().ok())
        {
            handle_netflix_report_no_error(&ctx, chat_id, user_id, id).await?;
        } else if data == "netflix:monthly_gift_claim" {
            handle_netflix_monthly_gift_claim(&ctx, chat_id, user_id, &lang).await?;
        }

        Ok(true)
    }
}

pub fn netflix_enabled(ctx: &AppContext) -> bool {
    config_bool(ctx, "netflix_enabled", true)
}

pub fn netflix_button_json(ctx: &AppContext, lang: &str) -> Value {
    let fallback_text = i18n::t(ctx, lang, "start_btn_netflix", "🎬 Xem Netflix");
    let configured_text = ctx.get_text("netflix_start_button_text", "");
    let text_source = if configured_text.trim().is_empty() {
        fallback_text.clone()
    } else {
        configured_text
    };
    let mut parts = i18n::button_parts_for_key(ctx, "start_btn_netflix", text_source);
    if parts.text.trim().is_empty() {
        let fallback_parts =
            i18n::button_parts_for_key(ctx, "start_btn_netflix", fallback_text.clone());
        if !fallback_parts.text.trim().is_empty() {
            parts.text = fallback_parts.text;
        }
    }
    if parts.text.trim().is_empty() {
        parts.text = netflix_default_button_label(lang).to_string();
    }

    if let Some(icon_id) = normalize_custom_emoji_id_value(
        &ctx.get_text("netflix_start_button_custom_emoji_id", ""),
    ) {
        parts.icon_custom_emoji_id = Some(icon_id);
    }

    let mut button = json!({
        "text": parts.text,
        "callback_data": "netflix:menu",
    });
    if let Some(icon_id) = parts.icon_custom_emoji_id
        && let Some(obj) = button.as_object_mut()
    {
        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
    }
    button
}

pub async fn notify_monthly_gift_if_eligible(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    _lang: &str,
) -> Result<()> {
    if !netflix_monthly_gift_enabled(ctx) || !netflix_enabled(ctx) {
        return Ok(());
    }
    if !user_has_unclaimed_monthly_gift(ctx, user_id).await? {
        return Ok(());
    }

    ctx.bot
        .send_message(
            chat_id,
            netflix_text(
                ctx,
                "netflix_monthly_gift_notice",
                "🎁 Bạn đủ điều kiện nhận 1 vé xem Netflix miễn phí tháng này.\nĐiều kiện: nạp ví từ 200.000đ hoặc mua hàng tổng từ 200.000đ trong tháng.",
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            netflix_text(
                ctx,
                "netflix_monthly_gift_claim_button",
                "🎁 Nhận vé Netflix 1 tháng",
            ),
            "netflix:monthly_gift_claim",
        )]]))
        .await?;
    Ok(())
}

fn netflix_default_button_label(lang: &str) -> &'static str {
    if lang.to_ascii_lowercase().starts_with("vi") {
        "Xem Netflix"
    } else {
        "Watch Netflix"
    }
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
    if let Some(button) = netflix_reopen_latest_button(ctx) {
        menu_rows.push(vec![button]);
    }
    push_button_pair_row(
        &mut menu_rows,
        netflix_pc_guide_button(ctx),
        netflix_mobile_guide_button(ctx),
    );
    push_button_pair_row(
        &mut menu_rows,
        netflix_language_vi_guide_button(ctx),
        netflix_mobile_language_guide_button(ctx),
    );
    push_button_pair_row(
        &mut menu_rows,
        Some(i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_wallet",
            "💳 Ví tiền",
            "start:wallet",
        )),
        Some(InlineKeyboardButton::callback("⬅️ Quay lại", "start:menu")),
    );

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
            let session_id = save_netflix_session(ctx, user_id, chat_id.0, &cookie, price).await?;
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

async fn handle_netflix_monthly_gift_claim(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    _lang: &str,
) -> Result<()> {
    if !netflix_monthly_gift_enabled(ctx) || !netflix_enabled(ctx) {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_monthly_gift_unavailable_message",
                    "⚠️ Vé Netflix miễn phí hiện chưa khả dụng.",
                ),
            )
            .await?;
        return Ok(());
    }
    if !user_has_unclaimed_monthly_gift(ctx, user_id).await? {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_monthly_gift_not_eligible_message",
                    "⚠️ Bạn chưa đủ điều kiện hoặc đã nhận vé Netflix miễn phí tháng này.",
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

    let month_key = current_month_key();
    if !reserve_monthly_gift_claim(ctx, user_id, &month_key).await? {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_monthly_gift_not_eligible_message",
                    "⚠️ Bạn chưa đủ điều kiện hoặc đã nhận vé Netflix miễn phí tháng này.",
                ),
            )
            .await?;
        return Ok(());
    }

    ctx.bot
        .send_message(
            chat_id,
            netflix_text(
                ctx,
                "netflix_monthly_gift_loading_message",
                "⏳ Đang lấy vé Netflix miễn phí cho bạn...",
            ),
        )
        .await?;

    match call_get_cookie_api(ctx, &api_key).await {
        Ok(cookie) => {
            let session_id = save_netflix_session(ctx, user_id, chat_id.0, &cookie, 0).await?;
            complete_monthly_gift_claim(ctx, user_id, &month_key, session_id).await?;
            send_netflix_cookie(ctx, chat_id, session_id, &cookie, 0).await?;
        }
        Err(err) => {
            release_monthly_gift_claim(ctx, user_id, &month_key).await?;
            tracing::error!("monthly netflix gift claim failed for user {user_id}: {err:#}");
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
                        "netflix:monthly_gift_claim",
                    ),
                    InlineKeyboardButton::callback("⬅️ Menu Netflix", "netflix:menu"),
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

async fn handle_netflix_reopen_latest(
    ctx: &AppContext,
    chat_id: ChatId,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    if netflix_api_key(ctx).is_none() {
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
    }

    let Some(session) = get_latest_netflix_session(ctx, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_reopen_latest_missing_message",
                    "⚠️ Chưa có lượt Netflix cũ để mở lại. Hãy lấy Netflix trước.",
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback(
                    netflix_text(ctx, "netflix_buy_button_text", "🎬 Lấy Netflix"),
                    "netflix:buy",
                ),
                InlineKeyboardButton::callback("⬅️ Quay lại", "netflix:menu"),
            ]]))
            .await?;
        return Ok(());
    };

    handle_netflix_regen(ctx, chat_id, user_id, session.id, lang).await
}

async fn send_netflix_cookie(
    ctx: &AppContext,
    chat_id: ChatId,
    session_id: i64,
    cookie: &NetflixCookie,
    price: i64,
) -> Result<()> {
    let mut rows = Vec::new();
    push_button_pair_row(
        &mut rows,
        cookie.pc_login_link.as_deref().and_then(url_button_link).map(|link| {
            InlineKeyboardButton::url(
                netflix_text(ctx, "netflix_pc_button_text", "💻 Mở PC"),
                link,
            )
        }),
        cookie.mobile_login_link.as_deref().and_then(url_button_link).map(|link| {
            InlineKeyboardButton::url(
                netflix_text(ctx, "netflix_mobile_button_text", "📱 Mở Mobile"),
                link,
            )
        }),
    );
    push_button_pair_row(
        &mut rows,
        Some(InlineKeyboardButton::callback(
            netflix_text(ctx, "netflix_cookie_button_text", "🍪 Lấy cookie"),
            format!("netflix:cookie:{session_id}"),
        )),
        Some(InlineKeyboardButton::callback(
            netflix_text(ctx, "netflix_regen_button_text", "🔄 Tạo lại link"),
            format!("netflix:regen:{session_id}"),
        )),
    );
    rows.push(vec![InlineKeyboardButton::callback(
        netflix_text(ctx, "netflix_report_cookie_button_text", "⚠️ Báo cookie lỗi"),
        format!("netflix:report:{session_id}"),
    )]);
    push_button_pair_row(
        &mut rows,
        netflix_pc_guide_button(ctx),
        netflix_language_vi_guide_button(ctx),
    );
    push_button_pair_row(
        &mut rows,
        netflix_mobile_guide_button(ctx),
        netflix_mobile_language_guide_button(ctx),
    );
    push_button_pair_row(
        &mut rows,
        Some(InlineKeyboardButton::callback(
            netflix_text(ctx, "netflix_buy_again_button_text", "🎬 Lấy Netflix khác"),
            "netflix:buy",
        )),
        Some(InlineKeyboardButton::callback("⬅️ Menu Netflix", "netflix:menu")),
    );

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

fn push_button_pair_row(
    rows: &mut Vec<Vec<InlineKeyboardButton>>,
    left: Option<InlineKeyboardButton>,
    right: Option<InlineKeyboardButton>,
) {
    match (left, right) {
        (Some(left), Some(right)) => rows.push(vec![left, right]),
        (Some(left), None) => rows.push(vec![left]),
        (None, Some(right)) => rows.push(vec![right]),
        (None, None) => {}
    }
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

async fn handle_netflix_cookie_report(
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

    let report = create_or_reopen_netflix_report(ctx, &session).await?;
    if report.status == "refunded" {
        ctx.bot
            .send_message(
                chat_id,
                netflix_text(
                    ctx,
                    "netflix_report_already_refunded_message",
                    "✅ Lượt Netflix này đã được hoàn tiền trước đó.",
                ),
            )
            .await?;
        return Ok(());
    }

    let sent = notify_admins_netflix_report(ctx, &report).await?;
    let message_key = if sent {
        "netflix_report_sent_message"
    } else {
        "netflix_report_no_admin_message"
    };
    let fallback = if sent {
        "✅ Đã báo admin kiểm tra cookie. Bạn vui lòng chờ phản hồi."
    } else {
        "⚠️ Chưa cấu hình admin nhận báo lỗi Netflix. Vui lòng liên hệ hỗ trợ."
    };
    ctx.bot
        .send_message(chat_id, netflix_text(ctx, message_key, fallback))
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                netflix_text(ctx, "netflix_regen_button_text", "🔄 Tạo lại link"),
                format!("netflix:regen:{session_id}"),
            ),
            InlineKeyboardButton::callback("⬅️ Menu Netflix", "netflix:menu"),
        ]]))
        .await?;

    Ok(())
}

async fn handle_netflix_report_refund(
    ctx: &AppContext,
    admin_chat_id: ChatId,
    admin_user_id: i64,
    report_id: i64,
) -> Result<()> {
    if !ctx.is_telegram_admin(admin_user_id) {
        ctx.bot
            .send_message(admin_chat_id, "Bạn không có quyền admin.")
            .await?;
        return Ok(());
    }

    let Some(report) = get_netflix_report(ctx, report_id).await? else {
        ctx.bot
            .send_message(admin_chat_id, "Không tìm thấy báo lỗi Netflix.")
            .await?;
        return Ok(());
    };
    if report.status == "refunded" {
        ctx.bot
            .send_message(admin_chat_id, "Báo lỗi này đã hoàn tiền rồi.")
            .await?;
        return Ok(());
    }

    let amount = netflix_report_refund_amount(ctx, &report.session);
    let balance_after = if amount > 0 {
        refund_netflix_report(ctx, &report, admin_user_id, amount).await?
    } else {
        mark_netflix_report_status(ctx, report.id, "refunded", admin_user_id, Some(0)).await?;
        wallet_repo::get_or_create_wallet(&ctx.pool, report.session.user_id)
            .await?
            .balance
    };

    ctx.bot
        .send_message(
            ChatId(report.session.chat_id),
            format!(
                "{}\n{}: <b>{}</b>\n{}: <b>{}</b>",
                netflix_text(
                    ctx,
                    "netflix_report_refund_user_message",
                    "✅ Admin xác nhận cookie lỗi và đã hoàn tiền vào ví của bạn."
                ),
                netflix_text(ctx, "netflix_report_refund_amount_label", "Số tiền hoàn"),
                format_vnd(amount),
                netflix_text(ctx, "netflix_report_balance_after_label", "Số dư ví"),
                format_vnd(balance_after)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                netflix_text(ctx, "netflix_buy_button_text", "🎬 Lấy Netflix"),
                "netflix:buy",
            ),
            InlineKeyboardButton::callback("💳 Ví tiền", "start:wallet"),
        ]]))
        .await?;

    ctx.bot
        .send_message(
            admin_chat_id,
            format!(
                "Đã hoàn {} cho user {}.",
                format_vnd(amount),
                report.session.user_id
            ),
        )
        .await?;
    Ok(())
}

async fn handle_netflix_report_no_error(
    ctx: &AppContext,
    admin_chat_id: ChatId,
    admin_user_id: i64,
    report_id: i64,
) -> Result<()> {
    if !ctx.is_telegram_admin(admin_user_id) {
        ctx.bot
            .send_message(admin_chat_id, "Bạn không có quyền admin.")
            .await?;
        return Ok(());
    }

    let Some(report) = get_netflix_report(ctx, report_id).await? else {
        ctx.bot
            .send_message(admin_chat_id, "Không tìm thấy báo lỗi Netflix.")
            .await?;
        return Ok(());
    };
    if report.status == "refunded" {
        ctx.bot
            .send_message(admin_chat_id, "Báo lỗi này đã hoàn tiền rồi, không thể báo không lỗi.")
            .await?;
        return Ok(());
    }
    mark_netflix_report_status(ctx, report.id, "no_error", admin_user_id, None).await?;

    ctx.bot
        .send_message(
            ChatId(report.session.chat_id),
            netflix_text(
                ctx,
                "netflix_report_no_error_user_message",
                "✅ Admin đã kiểm tra và cookie không lỗi. Vui lòng bấm Tạo lại link hoặc Mở lại link cũ để lấy link mới rồi xem lại.",
            ),
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                netflix_text(ctx, "netflix_regen_button_text", "🔄 Tạo lại link"),
                format!("netflix:regen:{}", report.session.id),
            ),
            InlineKeyboardButton::callback(
                netflix_text(ctx, "netflix_reopen_latest_button_text", "🔄 Mở lại link cũ"),
                "netflix:reopen_latest",
            ),
        ]]))
        .await?;

    ctx.bot
        .send_message(
            admin_chat_id,
            format!("Đã báo user {}: cookie không lỗi.", report.session.user_id),
        )
        .await?;
    Ok(())
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

fn netflix_reopen_latest_button(ctx: &AppContext) -> Option<InlineKeyboardButton> {
    if !config_bool(ctx, "netflix_reopen_latest_enabled", true) {
        return None;
    }
    Some(InlineKeyboardButton::callback(
        netflix_text(
            ctx,
            "netflix_reopen_latest_button_text",
            "🔄 Mở lại link cũ",
        ),
        "netflix:reopen_latest",
    ))
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
            purchase_amount INTEGER,
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

    let _ = sqlx::query("ALTER TABLE netflix_sessions ADD COLUMN purchase_amount INTEGER")
        .execute(pool)
        .await;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS netflix_cookie_reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL UNIQUE,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            admin_id INTEGER,
            admin_note TEXT,
            refunded_amount INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS netflix_monthly_gifts (
            user_id INTEGER NOT NULL,
            month_key TEXT NOT NULL,
            session_id INTEGER,
            claimed_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, month_key)
        )"#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn get_latest_netflix_session(
    ctx: &AppContext,
    user_id: i64,
    chat_id: i64,
) -> Result<Option<NetflixSession>> {
    let row = sqlx::query_as::<_, (
        i64,
        i64,
        i64,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    )>(
        r#"SELECT id, user_id, chat_id, log_id, cookie_number, cookie, pc_login_link, mobile_login_link, purchase_amount
        FROM netflix_sessions
        WHERE user_id = ? AND chat_id = ?
        ORDER BY id DESC
        LIMIT 1"#,
    )
    .bind(user_id)
    .bind(chat_id)
    .fetch_optional(&ctx.pool)
    .await?;

    Ok(row.map(netflix_session_from_row))
}

async fn save_netflix_session(
    ctx: &AppContext,
    user_id: i64,
    chat_id: i64,
    cookie: &NetflixCookie,
    purchase_amount: i64,
) -> Result<i64> {
    let id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO netflix_sessions
        (user_id, chat_id, log_id, cookie_number, cookie, pc_login_link, mobile_login_link, token_expires, purchase_amount, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
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
    .bind(purchase_amount)
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
    let row = sqlx::query_as::<_, (
        i64,
        i64,
        i64,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    )>(
        r#"SELECT id, user_id, chat_id, log_id, cookie_number, cookie, pc_login_link, mobile_login_link, purchase_amount
        FROM netflix_sessions
        WHERE id = ? AND user_id = ? AND chat_id = ?"#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind(chat_id)
    .fetch_optional(&ctx.pool)
    .await?;

    Ok(row.map(netflix_session_from_row))
}

fn netflix_session_from_row(
    row: (
        i64,
        i64,
        i64,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    ),
) -> NetflixSession {
    let (
        id,
        user_id,
        chat_id,
        log_id,
        cookie_number,
        cookie,
        pc_login_link,
        mobile_login_link,
        purchase_amount,
    ) = row;
    NetflixSession {
        id,
        user_id,
        chat_id,
        log_id,
        cookie_number,
        cookie,
        pc_login_link,
        mobile_login_link,
        purchase_amount,
    }
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

async fn create_or_reopen_netflix_report(
    ctx: &AppContext,
    session: &NetflixSession,
) -> Result<NetflixCookieReport> {
    let (id, status) = sqlx::query_as::<_, (i64, String)>(
        r#"INSERT INTO netflix_cookie_reports (session_id, user_id, chat_id, status, updated_at)
        VALUES (?, ?, ?, 'pending', datetime('now'))
        ON CONFLICT(session_id) DO UPDATE SET
            status = CASE
                WHEN netflix_cookie_reports.status = 'refunded' THEN netflix_cookie_reports.status
                ELSE 'pending'
            END,
            updated_at = datetime('now')
        RETURNING id, status"#,
    )
    .bind(session.id)
    .bind(session.user_id)
    .bind(session.chat_id)
    .fetch_one(&ctx.pool)
    .await?;

    Ok(NetflixCookieReport {
        id,
        session: session.clone(),
        status,
    })
}

async fn get_netflix_report(ctx: &AppContext, report_id: i64) -> Result<Option<NetflixCookieReport>> {
    let row = sqlx::query_as::<_, (
        i64,
        String,
        i64,
        i64,
        i64,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    )>(
        r#"SELECT r.id, r.status, s.id, s.user_id, s.chat_id, s.log_id, s.cookie_number,
            s.cookie, s.pc_login_link, s.mobile_login_link, s.purchase_amount
        FROM netflix_cookie_reports r
        JOIN netflix_sessions s ON s.id = r.session_id
        WHERE r.id = ?"#,
    )
    .bind(report_id)
    .fetch_optional(&ctx.pool)
    .await?;

    Ok(row.map(
        |(
            id,
            status,
            session_id,
            user_id,
            chat_id,
            log_id,
            cookie_number,
            cookie,
            pc_login_link,
            mobile_login_link,
            purchase_amount,
        )| NetflixCookieReport {
            id,
            status,
            session: NetflixSession {
                id: session_id,
                user_id,
                chat_id,
                log_id,
                cookie_number,
                cookie,
                pc_login_link,
                mobile_login_link,
                purchase_amount,
            },
        },
    ))
}

async fn notify_admins_netflix_report(
    ctx: &AppContext,
    report: &NetflixCookieReport,
) -> Result<bool> {
    let admin_ids = netflix_admin_ids(ctx);
    if admin_ids.is_empty() {
        return Ok(false);
    }

    let mut rows = Vec::new();
    push_button_pair_row(
        &mut rows,
        report
            .session
            .pc_login_link
            .as_deref()
            .and_then(url_button_link)
            .map(|link| {
                InlineKeyboardButton::url(
                    netflix_text(ctx, "netflix_report_admin_open_pc_button", "💻 Mở PC"),
                    link,
                )
            }),
        report
            .session
            .mobile_login_link
            .as_deref()
            .and_then(url_button_link)
            .map(|link| {
                InlineKeyboardButton::url(
                    netflix_text(ctx, "netflix_report_admin_open_mobile_button", "📱 Mở Mobile"),
                    link,
                )
            }),
    );
    rows.push(vec![
        InlineKeyboardButton::callback(
            netflix_text(ctx, "netflix_report_admin_refund_button", "💸 Cookie lỗi - Hoàn tiền"),
            format!("netflixreport:refund:{}", report.id),
        ),
        InlineKeyboardButton::callback(
            netflix_text(ctx, "netflix_report_admin_no_error_button", "✅ Không lỗi"),
            format!("netflixreport:ok:{}", report.id),
        ),
    ]);

    let text = format!(
        "{}\nReport ID: {}\nSession ID: {}\nUser: {}\nChat: {}\nCookie số: {}\nLog ID: <code>{}</code>\nGiá đã trừ: <b>{}</b>\n\nAdmin mở PC/Mobile để kiểm tra. Nếu cookie lỗi thì bấm hoàn tiền; nếu không lỗi thì bấm không lỗi để nhắn user tạo lại link.",
        netflix_text(
            ctx,
            "netflix_report_admin_title",
            "⚠️ USER BÁO COOKIE NETFLIX LỖI"
        ),
        report.id,
        report.session.id,
        report.session.user_id,
        report.session.chat_id,
        report
            .session
            .cookie_number
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        html_escape(&report.session.log_id),
        format_vnd(netflix_report_refund_amount(ctx, &report.session))
    );

    let mut sent = false;
    for admin_id in admin_ids {
        if let Err(err) = ctx.bot
            .send_message(ChatId(admin_id), text.clone())
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(rows.clone()))
            .await
        {
            tracing::warn!("send netflix cookie report to admin {admin_id} failed: {err}");
        } else {
            sent = true;
        }
    }
    Ok(sent)
}

async fn mark_netflix_report_status(
    ctx: &AppContext,
    report_id: i64,
    status: &str,
    admin_id: i64,
    refunded_amount: Option<i64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE netflix_cookie_reports
        SET status = ?, admin_id = ?, refunded_amount = COALESCE(?, refunded_amount), updated_at = datetime('now')
        WHERE id = ? AND status != 'refunded'"#,
    )
    .bind(status)
    .bind(admin_id)
    .bind(refunded_amount)
    .bind(report_id)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

async fn refund_netflix_report(
    ctx: &AppContext,
    report: &NetflixCookieReport,
    admin_id: i64,
    amount: i64,
) -> Result<i64> {
    let mut tx = ctx.pool.begin().await?;
    let updated_report_id = sqlx::query_scalar::<_, i64>(
        r#"UPDATE netflix_cookie_reports
        SET status = 'refunded', admin_id = ?, refunded_amount = ?, updated_at = datetime('now')
        WHERE id = ? AND status != 'refunded'
        RETURNING id"#,
    )
    .bind(admin_id)
    .bind(amount)
    .bind(report.id)
    .fetch_optional(&mut *tx)
    .await?;

    if updated_report_id.is_none() {
        tx.rollback().await?;
        return Ok(wallet_repo::get_or_create_wallet(&ctx.pool, report.session.user_id)
            .await?
            .balance);
    }

    let note = format!("Hoàn tiền Netflix cookie lỗi report #{}", report.id);
    let refund_order_id = format!("netflix-report-{}", report.id);
    let balance_after = wallet_repo::credit_wallet(
        &mut tx,
        report.session.user_id,
        amount,
        "refund",
        Some(&refund_order_id),
        None,
        Some(&note),
    )
    .await?;
    tx.commit().await?;
    Ok(balance_after)
}

fn netflix_report_refund_amount(ctx: &AppContext, session: &NetflixSession) -> i64 {
    session.purchase_amount.unwrap_or_else(|| netflix_price(ctx)).max(0)
}

fn netflix_admin_ids(ctx: &AppContext) -> Vec<i64> {
    let mut ids = ctx.order_notification_admin_ids();
    for admin_id in ctx.telegram_icon_admin_ids() {
        if !ids.iter().any(|existing| *existing == admin_id) {
            ids.push(admin_id);
        }
    }
    ids
}

async fn user_has_unclaimed_monthly_gift(ctx: &AppContext, user_id: i64) -> Result<bool> {
    let month_key = current_month_key();
    let claimed: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(1)
        FROM netflix_monthly_gifts
        WHERE user_id = ? AND month_key = ? AND claimed_at IS NOT NULL"#,
    )
    .bind(user_id)
    .bind(&month_key)
    .fetch_one(&ctx.pool)
    .await?;
    if claimed > 0 {
        return Ok(false);
    }

    let (month_start, next_month_start) = current_month_bounds();
    let promo_start = netflix_monthly_gift_start_at(ctx);
    let topup_total: i64 = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(amount), 0)
        FROM wallet_transactions
        WHERE user_id = ?
          AND type = 'topup'
          AND amount > 0
          AND julianday(created_at) >= julianday(?)
          AND julianday(created_at) >= julianday(?)
          AND julianday(created_at) < julianday(?)"#,
    )
    .bind(user_id)
    .bind(&month_start)
    .bind(&promo_start)
    .bind(&next_month_start)
    .fetch_one(&ctx.pool)
    .await?;

    let purchase_total: i64 = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(amount), 0)
        FROM orders
        WHERE user_id = ?
          AND status = 'paid'
          AND julianday(COALESCE(paid_at, created_at)) >= julianday(?)
          AND julianday(COALESCE(paid_at, created_at)) >= julianday(?)
          AND julianday(COALESCE(paid_at, created_at)) < julianday(?)"#,
    )
    .bind(user_id)
    .bind(&month_start)
    .bind(&promo_start)
    .bind(&next_month_start)
    .fetch_one(&ctx.pool)
    .await?;

    let threshold = netflix_monthly_gift_threshold(ctx);
    Ok(threshold > 0 && (topup_total >= threshold || purchase_total >= threshold))
}

async fn reserve_monthly_gift_claim(
    ctx: &AppContext,
    user_id: i64,
    month_key: &str,
) -> Result<bool> {
    sqlx::query(
        r#"DELETE FROM netflix_monthly_gifts
        WHERE user_id = ? AND month_key = ? AND claimed_at IS NULL"#,
    )
    .bind(user_id)
    .bind(month_key)
    .execute(&ctx.pool)
    .await?;

    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO netflix_monthly_gifts
        (user_id, month_key, created_at, updated_at)
        VALUES (?, ?, datetime('now'), datetime('now'))"#,
    )
    .bind(user_id)
    .bind(month_key)
    .execute(&ctx.pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn complete_monthly_gift_claim(
    ctx: &AppContext,
    user_id: i64,
    month_key: &str,
    session_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE netflix_monthly_gifts
        SET session_id = ?, claimed_at = datetime('now'), updated_at = datetime('now')
        WHERE user_id = ? AND month_key = ?"#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind(month_key)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

async fn release_monthly_gift_claim(
    ctx: &AppContext,
    user_id: i64,
    month_key: &str,
) -> Result<()> {
    sqlx::query(
        r#"DELETE FROM netflix_monthly_gifts
        WHERE user_id = ? AND month_key = ? AND claimed_at IS NULL"#,
    )
    .bind(user_id)
    .bind(month_key)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

fn netflix_monthly_gift_enabled(ctx: &AppContext) -> bool {
    config_bool(ctx, "netflix_monthly_gift_enabled", true)
}

fn netflix_monthly_gift_threshold(ctx: &AppContext) -> i64 {
    ctx.get_text("netflix_monthly_gift_threshold", "200000")
        .trim()
        .parse::<i64>()
        .unwrap_or(200_000)
        .max(0)
}

fn netflix_monthly_gift_start_at(ctx: &AppContext) -> String {
    netflix_text(
        ctx,
        "netflix_monthly_gift_start_at",
        MONTHLY_GIFT_START_AT_DEFAULT,
    )
}

fn current_month_key() -> String {
    let now = Utc::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

fn current_month_bounds() -> (String, String) {
    let now = Utc::now();
    let month_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    let (next_year, next_month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let next_month_start = Utc
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    (month_start.to_rfc3339(), next_month_start.to_rfc3339())
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
            "💻 Xem trên PC",
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
            "🌐 Đổi ngôn ngữ PC",
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
            "📱 Xem trên Mobile",
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
            "🌐 Đổi ngôn ngữ Mobile",
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

fn normalize_custom_emoji_id_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        Some(trimmed.to_string())
    } else {
        None
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

    fn test_ctx(configs: std::collections::HashMap<String, String>) -> Arc<AppContext> {
        let config = crate::config::Config::from_env_map(&std::collections::HashMap::from([
            ("TELEGRAM_TOKEN".to_string(), "test".to_string()),
            ("DATABASE_URL".to_string(), "sqlite::memory:".to_string()),
            ("WEBHOOK_SECRET".to_string(), "secret".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "12345678901234567890123456789012".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup".to_string()),
        ]))
        .unwrap();

        AppContext::new(
            Bot::new("test"),
            sqlx::sqlite::SqlitePoolOptions::new()
                .connect_lazy("sqlite::memory:")
                .unwrap(),
            config,
            configs,
            crate::bot::texts::BotTexts::default(),
            vec![],
        )
    }

    #[test]
    fn config_bool_accepts_admin_toggle_values() {
        let ctx = test_ctx(std::collections::HashMap::from([(
            "netflix_enabled".to_string(),
            "bật".to_string(),
        )]));

        assert!(netflix_enabled(&ctx));
    }

    #[test]
    fn netflix_button_keeps_label_when_custom_emoji_text_is_icon_only() {
        let ctx = test_ctx(std::collections::HashMap::from([
            (
                "netflix_start_button_text".to_string(),
                "🎬".to_string(),
            ),
            (
                "netflix_start_button_custom_emoji_id".to_string(),
                "5368324170671202286".to_string(),
            ),
        ]));

        let button = netflix_button_json(&ctx, "vi");
        assert_eq!(button["text"], "Xem Netflix");
        assert_eq!(button["callback_data"], "netflix:menu");
        assert_eq!(button["icon_custom_emoji_id"], "5368324170671202286");
    }
}
