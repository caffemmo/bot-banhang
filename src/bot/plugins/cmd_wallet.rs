use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rand::{Rng, distributions::Alphanumeric};
use teloxide::payloads::{EditMessageCaptionSetters, SendPhotoSetters};
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ForceReply, InlineKeyboardButton, InlineKeyboardMarkup, InputFile,
    Message, MessageId, ParseMode,
};
use tokio::time::{Duration as TokioDuration, sleep};
use url::Url;

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::core::qr::vietqr_link;
use crate::domains::crypto_pay::bep20 as bep20_pay;
use crate::domains::crypto_pay::binance as binance_pay;
use crate::domains::crypto_pay::binance_worker;
use crate::domains::crypto_pay::models::{
    CryptoPaymentMethod, CryptoPaymentRequest, CryptoPaymentStatus,
};
use crate::domains::crypto_pay::repo as crypto_repo;
use crate::domains::orders::webhook::TOPUP_TTL_MINUTES;
use crate::domains::wallet::repo as wallet_repo;
use tracing::warn;

const COUNTDOWN_TICK_SECONDS: u64 = 2;
const QUICK_TOPUP_AMOUNTS: [i64; 3] = [10_000, 30_000, 50_000];

fn topup_expires_at(created_at: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&Utc) + Duration::minutes(TOPUP_TTL_MINUTES))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
                .map(|dt| dt + Duration::minutes(TOPUP_TTL_MINUTES))
        })
        .unwrap_or_else(|_| Utc::now() + Duration::minutes(TOPUP_TTL_MINUTES))
}

fn mmss(remaining_secs: i64) -> String {
    let clamped = remaining_secs.max(0);
    let mins = clamped / 60;
    let secs = clamped % 60;
    format!("{mins:02}:{secs:02}")
}

fn render_qr_caption(base_caption: &str, countdown_line: &str) -> String {
    format!("{base_caption}\n{countdown_line}")
}

fn topup_amount_prompt(ctx: &AppContext, lang: &str) -> String {
    i18n::t(
        ctx,
        lang,
        "topup_amount_prompt",
        "💰 Enter the amount to top up (example: 100000 = 100,000 VND):\n\nMinimum 1,000 VND, maximum 100,000,000 VND.",
    )
}

pub(crate) async fn prompt_topup_amount(
    ctx: &Arc<AppContext>,
    chat_id: ChatId,
    dialogue: BotDialogue,
    lang: &str,
) -> Result<()> {
    dialogue.update(State::TopupEnterAmount).await?;
    ctx.bot
        .send_message(chat_id, topup_amount_prompt(ctx, lang))
        .reply_markup(topup_amount_keyboard(ctx, lang))
        .await?;
    Ok(())
}

pub(crate) async fn show_topup_history(
    ctx: &Arc<AppContext>,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    let topups = sqlx::query_as::<_, crate::domains::wallet::models::WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests
         WHERE user_id = ?
         ORDER BY created_at DESC, id DESC
         LIMIT 10",
    )
    .bind(user_id)
    .fetch_all(&ctx.pool)
    .await?;

    let mut text = i18n::t(
        ctx,
        lang,
        "wallet_history_header",
        "🕒 TOP-UP HISTORY\n\nYour recent top-up requests:\n",
    );

    if topups.is_empty() {
        text.push_str(&format!(
            "\n<i>{}</i>",
            i18n::t(
                ctx,
                lang,
                "wallet_history_empty",
                "You have no top-up requests."
            )
        ));
    } else {
        text.push('\n');
        for req in &topups {
            let status_icon = match req.status.as_str() {
                "completed" => "✅",
                "expired" => "❌",
                _ => "⏳",
            };
            let status_label = match req.status.as_str() {
                "completed" => i18n::t(ctx, lang, "wallet_status_completed", "Completed"),
                "expired" => i18n::t(ctx, lang, "wallet_status_expired", "Cancelled"),
                _ => i18n::t(ctx, lang, "wallet_status_pending", "Pending"),
            };
            let date_str = if req.created_at.len() >= 16 {
                &req.created_at[..16]
            } else {
                &req.created_at
            };
            text.push_str(&format!(
                "{} <code>{}</code> — <b>{}</b> — {} (<i>{}</i>)\n",
                status_icon,
                req.memo,
                format_vnd(req.amount),
                status_label,
                date_str
            ));
        }
    }

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Back"),
            "wallet:show",
        ),
        InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "wallet_btn_topup", "💰 Top up"),
            "wallet:topup",
        ),
    ]]);

    if let Some(message_id) = message_id {
        ctx.bot
            .edit_message_text(chat_id, message_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    } else {
        ctx.bot
            .send_message(chat_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    }
    Ok(())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn copyable_code(value: &str) -> String {
    format!("<code>{}</code>", html_escape(value))
}

fn keep_qr_keyboard_on_caption_edit<T>(request: T, keyboard: &InlineKeyboardMarkup) -> T
where
    T: EditMessageCaptionSetters,
{
    request.reply_markup(keyboard.clone())
}

async fn edit_caption_soft(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    message_id: MessageId,
    caption: String,
    keyboard: &InlineKeyboardMarkup,
) -> Result<()> {
    for attempt in 0..4_u64 {
        let request = ctx
            .bot
            .edit_message_caption(chat_id, message_id)
            .caption(caption.clone())
            .parse_mode(ParseMode::Html);
        let result = keep_qr_keyboard_on_caption_edit(request, keyboard).await;
        match result {
            Ok(_) => return Ok(()),
            Err(err) => {
                let err_msg = err.to_string().to_lowercase();
                if err_msg.contains("message is not modified") {
                    return Ok(());
                }

                let is_retryable = err_msg.contains("too many requests")
                    || err_msg.contains("retry after")
                    || err_msg.contains("timeout")
                    || err_msg.contains("network")
                    || err_msg.contains("temporar");
                if is_retryable && attempt < 3 {
                    sleep(TokioDuration::from_millis(300 * (attempt + 1))).await;
                    continue;
                }

                return Err(err.into());
            }
        }
    }
    Ok(())
}

fn spawn_topup_qr_countdown(
    ctx: Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    message_id: MessageId,
    topup_id: i64,
    base_caption: String,
    expires_at: DateTime<Utc>,
    keyboard: InlineKeyboardMarkup,
    lang: String,
) {
    tokio::spawn(async move {
        loop {
            let status = sqlx::query_scalar::<_, String>(
                "SELECT status FROM wallet_topup_requests WHERE id = ?",
            )
            .bind(topup_id)
            .fetch_optional(&ctx.pool)
            .await
            .ok()
            .flatten();
            let Some(status) = status else { break };
            if status != "pending" {
                break;
            }

            let now = Utc::now();
            if now >= expires_at {
                let _ = edit_caption_soft(
                    &ctx,
                    chat_id,
                    message_id,
                    render_qr_caption(
                        &base_caption,
                        &i18n::t(&ctx, &lang, "qr_expired", "⛔ QR has expired."),
                    ),
                    &keyboard,
                )
                .await;
                break;
            }

            let remaining = (expires_at - now).num_seconds();
            let line = i18n::tr(
                &ctx,
                &lang,
                "qr_countdown",
                "⏰ QR valid for: {time}",
                &[("time", mmss(remaining))],
            );
            if let Err(err) = edit_caption_soft(
                &ctx,
                chat_id,
                message_id,
                render_qr_caption(&base_caption, &line),
                &keyboard,
            )
            .await
            {
                warn!("failed to update topup QR countdown {topup_id}: {err}");
            }
            sleep(TokioDuration::from_secs(COUNTDOWN_TICK_SECONDS)).await;
        }
    });
}

pub struct WalletCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for WalletCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdWallet"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "wallet".to_string(),
            description: "View your wallet".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool> {
        let text = msg.text().unwrap_or("");
        let state = dialogue.get().await?.unwrap_or_default();

        match state {
            State::TopupEnterAmount => {
                handle_topup_amount(&ctx, msg, dialogue).await?;
                return Ok(true);
            }
            State::TopupUsdtEnterAmount => {
                handle_usdt_topup_amount(&ctx, msg, dialogue).await?;
                return Ok(true);
            }
            State::TopupBinanceEnterAmount => {
                handle_binance_topup_amount(&ctx, msg, dialogue).await?;
                return Ok(true);
            }
            State::Idle if text.starts_with("/wallet") => {
                let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
                show_wallet(&ctx, msg.chat.id, user_id).await?;
                return Ok(true);
            }
            _ => {}
        }

        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool> {
        let data = q.data.clone().unwrap_or_default();
        if data == "wallet:topup"
            || data == "wallet:topup_history"
            || data == "wallet:topup_usdt"
            || data == "wallet:topup_binance"
            || data == "wallet:show"
            || data.starts_with("topupamt:")
            || data.starts_with("topupusdtamt:")
            || data.starts_with("topupbinanceamt:")
            || data.starts_with("wallettopupcheck:")
            || data.starts_with("wallettopupcancel:")
            || data.starts_with("walletbinancecheck:")
            || data.starts_with("walletbinancecancel:")
            || data.starts_with("canceltopup:")
        {
            handle_wallet_callback(&ctx, q, dialogue).await?;
            return Ok(true);
        }
        Ok(false)
    }
}

// ──────────────────────────────────────────────
// Hiển thị ví
// ──────────────────────────────────────────────

// ──────────────────────────────────────────────
// Hiển thị ví
// ──────────────────────────────────────────────

async fn get_wallet_text_and_keyboard(
    ctx: &Arc<AppContext>,
    user_id: i64,
) -> Result<(String, InlineKeyboardMarkup)> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;

    let text = i18n::tr(
        ctx,
        &lang,
        "wallet_header",
        "💳 YOUR WALLET\n\nBalance: {balance}\n",
        &[("balance", format_vnd(wallet.balance))],
    );

    let mut rows = vec![vec![
        i18n::inline_button_callback(ctx, &lang, "wallet_btn_topup", "💰 Top up", "wallet:topup"),
        i18n::inline_button_callback(
            ctx,
            &lang,
            "wallet_btn_history",
            "📜 History",
            "wallet:topup_history",
        ),
        i18n::inline_button_callback(ctx, &lang, "start_btn_shop", "🛒 Shop", "start:shop"),
    ]];
    if ctx.bep20_enabled() {
        rows.insert(
            0,
            vec![i18n::inline_button_callback(
                ctx,
                &lang,
                "wallet_btn_topup_usdt",
                "🟢 Top up USDT BEP20",
                "wallet:topup_usdt",
            )],
        );
    }
    if ctx.binance_pay_enabled() {
        rows.insert(
            0,
            vec![i18n::inline_button_callback(
                ctx,
                &lang,
                "wallet_btn_topup_binance",
                "🟡 Top up Binance Pay",
                "wallet:topup_binance",
            )],
        );
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        &lang,
        "back_btn",
        "⬅️ Quay lại",
        "start:menu",
    )]);
    let keyboard = InlineKeyboardMarkup::new(rows);

    Ok((text, keyboard))
}

pub async fn show_wallet(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
) -> Result<()> {
    let (text, keyboard) = get_wallet_text_and_keyboard(ctx, user_id).await?;

    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

// ──────────────────────────────────────────────
// Callback dispatcher
// ──────────────────────────────────────────────

async fn handle_wallet_callback(
    ctx: &Arc<AppContext>,
    q: CallbackQuery,
    dialogue: BotDialogue,
) -> Result<()> {
    let data = q.data.clone().unwrap_or_default();
    let _ = ctx.bot.answer_callback_query(q.id.clone()).await;

    let Some(ref msg) = q.message else {
        return Ok(());
    };
    let chat_id = msg.chat().id;
    let msg_id = msg.id();
    let user_id = q.from.id.0 as i64;
    let lang = i18n::user_lang(ctx, user_id, q.from.language_code.as_deref()).await;

    if data == "wallet:topup" {
        prompt_topup_amount(ctx, chat_id, dialogue, &lang).await?;
    } else if data == "wallet:topup_usdt" {
        dialogue.update(State::TopupUsdtEnterAmount).await?;
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_usdt_amount_prompt",
                    "🟢 Enter the VND amount you want to top up by USDT BEP20.",
                ),
            )
            .reply_markup(usdt_topup_amount_keyboard(ctx, &lang))
            .await?;
    } else if data == "wallet:topup_binance" {
        dialogue.update(State::TopupBinanceEnterAmount).await?;
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_binance_amount_prompt",
                    "🟡 Enter the VND amount you want to top up by Binance Pay.",
                ),
            )
            .reply_markup(binance_topup_amount_keyboard(ctx, &lang))
            .await?;
    } else if data == "wallet:topup_history" {
        show_topup_history(ctx, chat_id, Some(msg_id), user_id, &lang).await?;
    } else if data == "wallet:show" {
        let (text, keyboard) = get_wallet_text_and_keyboard(ctx, user_id).await?;
        ctx.bot
            .edit_message_text(chat_id, msg_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    } else if data == "topupamt:custom" {
        dialogue.update(State::TopupEnterAmount).await?;
        ctx.bot
            .send_message(chat_id, topup_amount_prompt(ctx, &lang))
            .reply_markup(custom_topup_amount_reply_markup(ctx, &lang))
            .await?;
    } else if let Some(amount_str) = data.strip_prefix("topupamt:") {
        let amount: i64 = amount_str.parse().unwrap_or(0);
        process_topup_amount(ctx, chat_id, user_id, amount, dialogue).await?;
    } else if let Some(amount_str) = data.strip_prefix("topupusdtamt:") {
        if amount_str == "custom" {
            dialogue.update(State::TopupUsdtEnterAmount).await?;
            ctx.bot
                .send_message(
                    chat_id,
                    i18n::t(
                        ctx,
                        &lang,
                        "topup_usdt_amount_prompt",
                        "🟢 Enter the VND amount you want to top up by USDT BEP20.",
                    ),
                )
                .reply_markup(custom_topup_amount_reply_markup(ctx, &lang))
                .await?;
        } else {
            let amount: i64 = amount_str.parse().unwrap_or(0);
            process_usdt_topup_amount(ctx, chat_id, user_id, amount, dialogue).await?;
        }
    } else if let Some(amount_str) = data.strip_prefix("topupbinanceamt:") {
        if amount_str == "custom" {
            dialogue.update(State::TopupBinanceEnterAmount).await?;
            ctx.bot
                .send_message(
                    chat_id,
                    i18n::t(
                        ctx,
                        &lang,
                        "topup_binance_amount_prompt",
                        "🟡 Enter the VND amount you want to top up by Binance Pay.",
                    ),
                )
                .reply_markup(custom_topup_amount_reply_markup(ctx, &lang))
                .await?;
        } else {
            let amount: i64 = amount_str.parse().unwrap_or(0);
            process_binance_topup_amount(ctx, chat_id, user_id, amount, dialogue).await?;
        }
    } else if let Some(payment_id) = data.strip_prefix("wallettopupcheck:") {
        handle_wallet_crypto_topup_check(ctx, chat_id, user_id, payment_id, false).await?;
    } else if let Some(payment_id) = data.strip_prefix("wallettopupcancel:") {
        handle_wallet_crypto_topup_cancel(ctx, chat_id, user_id, payment_id, false).await?;
    } else if let Some(payment_id) = data.strip_prefix("walletbinancecheck:") {
        handle_binance_topup_check(ctx, chat_id, user_id, payment_id).await?;
    } else if let Some(payment_id) = data.strip_prefix("walletbinancecancel:") {
        handle_binance_topup_cancel(ctx, chat_id, user_id, payment_id).await?;
    } else if let Some(topup_id_str) = data.strip_prefix("canceltopup:") {
        let topup_id: i64 = topup_id_str.parse().unwrap_or(0);
        handle_cancel_topup(ctx, chat_id, msg_id, user_id, topup_id).await?;
    }

    Ok(())
}

// ──────────────────────────────────────────────
// Nhập số tiền nạp
// ──────────────────────────────────────────────

async fn handle_topup_amount(
    ctx: &Arc<AppContext>,
    msg: Message,
    dialogue: BotDialogue,
) -> Result<()> {
    let raw = msg.text().unwrap_or("").trim().to_string();
    let amount: i64 = raw.replace('.', "").replace(',', "").parse().unwrap_or(0);
    let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);

    process_topup_amount(ctx, msg.chat.id, user_id, amount, dialogue).await?;

    Ok(())
}

async fn handle_usdt_topup_amount(
    ctx: &Arc<AppContext>,
    msg: Message,
    dialogue: BotDialogue,
) -> Result<()> {
    let raw = msg.text().unwrap_or("").trim().to_string();
    let amount: i64 = raw.replace('.', "").replace(',', "").parse().unwrap_or(0);
    let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);

    process_usdt_topup_amount(ctx, msg.chat.id, user_id, amount, dialogue).await?;

    Ok(())
}

async fn handle_binance_topup_amount(
    ctx: &Arc<AppContext>,
    msg: Message,
    dialogue: BotDialogue,
) -> Result<()> {
    let raw = msg.text().unwrap_or("").trim().to_string();
    let amount: i64 = raw.replace('.', "").replace(',', "").parse().unwrap_or(0);
    let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);

    process_binance_topup_amount(ctx, msg.chat.id, user_id, amount, dialogue).await?;

    Ok(())
}

fn topup_amount_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        QUICK_TOPUP_AMOUNTS
            .iter()
            .map(|amount| {
                InlineKeyboardButton::callback(
                    format!("{}k", amount / 1_000),
                    format!("topupamt:{amount}"),
                )
            })
            .collect(),
        vec![InlineKeyboardButton::callback(
            i18n::t(
                ctx,
                lang,
                "wallet_btn_custom_amount",
                "⌨️ Enter another amount",
            ),
            "topupamt:custom",
        )],
        vec![InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Quay lại"),
            "wallet:show",
        )],
    ])
}

fn usdt_topup_amount_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        QUICK_TOPUP_AMOUNTS
            .iter()
            .map(|amount| {
                InlineKeyboardButton::callback(
                    format!("{}k", amount / 1_000),
                    format!("topupusdtamt:{amount}"),
                )
            })
            .collect(),
        vec![InlineKeyboardButton::callback(
            i18n::t(
                ctx,
                lang,
                "wallet_btn_custom_amount",
                "⌨️ Enter another amount",
            ),
            "topupusdtamt:custom",
        )],
        vec![InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Quay lại"),
            "wallet:show",
        )],
    ])
}

fn binance_topup_amount_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        QUICK_TOPUP_AMOUNTS
            .iter()
            .map(|amount| {
                InlineKeyboardButton::callback(
                    format!("{}k", amount / 1_000),
                    format!("topupbinanceamt:{amount}"),
                )
            })
            .collect(),
        vec![InlineKeyboardButton::callback(
            i18n::t(
                ctx,
                lang,
                "wallet_btn_custom_amount",
                "⌨️ Enter another amount",
            ),
            "topupbinanceamt:custom",
        )],
        vec![InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Quay lại"),
            "wallet:show",
        )],
    ])
}

fn custom_topup_amount_reply_markup(ctx: &AppContext, lang: &str) -> ForceReply {
    ForceReply::new()
        .input_field_placeholder(i18n::t(
            ctx,
            lang,
            "topup_amount_placeholder",
            "Example: 100000",
        ))
        .selective()
}

fn topup_config_error(bank_account: &str) -> bool {
    bank_account.trim().is_empty()
}

async fn process_topup_amount(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    amount: i64,
    dialogue: BotDialogue,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    if amount < 1_000 {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_amount_min",
                    "⚠️ Minimum top-up amount is 1,000 VND. Please enter again.",
                ),
            )
            .reply_markup(topup_amount_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }
    if amount > 100_000_000 {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_amount_max",
                    "⚠️ Maximum top-up amount is 100,000,000 VND. Please enter again.",
                ),
            )
            .reply_markup(topup_amount_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    if topup_config_error(&ctx.bank_account()) {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_bank_not_configured",
                    "⚠️ Receiving bank account is not configured. Please contact admin.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    if let Some(pending) =
        wallet_repo::find_latest_pending_topup_by_user_id(&ctx.pool, user_id).await?
    {
        dialogue.update(State::Idle).await?;
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_pending_exists",
                    "⚠️ You already have an unfinished top-up request. Here is the current QR again.",
                ),
            )
            .await?;
        send_topup_qr(
            ctx,
            chat_id,
            &pending.memo,
            pending.amount,
            pending.id,
            &pending.created_at,
            &lang,
        )
        .await?;
        return Ok(());
    }

    let memo = generate_topup_memo(&ctx.pool).await?;
    let topup =
        wallet_repo::create_topup_request(&ctx.pool, user_id, chat_id.0, amount, &memo).await?;

    dialogue.update(State::Idle).await?;
    send_topup_qr(
        ctx,
        chat_id,
        &topup.memo,
        amount,
        topup.id,
        &topup.created_at,
        &lang,
    )
    .await?;
    Ok(())
}

async fn process_usdt_topup_amount(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    amount: i64,
    dialogue: BotDialogue,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    if amount < 1_000 || amount > 100_000_000 {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_amount_invalid",
                    "⚠️ Amount must be from 1,000 to 100,000,000 VND.",
                ),
            )
            .reply_markup(usdt_topup_amount_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    match bep20_pay::create_or_reuse_bep20_wallet_topup(ctx.clone(), user_id, chat_id.0, amount)
        .await
    {
        Ok(payment) => {
            dialogue.update(State::Idle).await?;
            send_usdt_topup_instructions(ctx, chat_id, &payment, &lang).await?;
        }
        Err(err) => {
            ctx.bot
                .send_message(
                    chat_id,
                    i18n::tr(
                        ctx,
                        &lang,
                        "topup_usdt_create_failed",
                        "Could not create USDT top-up request: {error}",
                        &[("error", err.to_string())],
                    ),
                )
                .reply_markup(wallet_back_keyboard(ctx, &lang))
                .await?;
        }
    }
    Ok(())
}

async fn process_binance_topup_amount(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    amount: i64,
    dialogue: BotDialogue,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    if amount < 1_000 || amount > 100_000_000 {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_amount_invalid",
                    "⚠️ Amount must be from 1,000 to 100,000,000 VND.",
                ),
            )
            .reply_markup(binance_topup_amount_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    match binance_pay::create_or_reuse_binance_wallet_topup(ctx.clone(), user_id, chat_id.0, amount)
        .await
    {
        Ok(payment) => {
            dialogue.update(State::Idle).await?;
            send_binance_topup_instructions(ctx, chat_id, &payment, &lang).await?;
        }
        Err(err) => {
            ctx.bot
                .send_message(
                    chat_id,
                    i18n::tr(
                        ctx,
                        &lang,
                        "topup_binance_create_failed",
                        "Could not create Binance Pay top-up request: {error}",
                        &[("error", err.to_string())],
                    ),
                )
                .reply_markup(wallet_back_keyboard(ctx, &lang))
                .await?;
        }
    }
    Ok(())
}

async fn send_usdt_topup_instructions(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    payment: &crate::domains::crypto_pay::models::CryptoPaymentRequest,
    lang: &str,
) -> Result<()> {
    let address = payment.address.as_deref().unwrap_or("");
    let amount = bep20_pay::format_bep20_amount(payment.amount_usdt_expected);
    let text = i18n::tr(
        ctx,
        lang,
        "topup_usdt_instructions",
        "🟢 USDT BEP20 TOP-UP\n\nTop-up value: {amount_vnd}\nUSDT amount to send: {amount_usdt} USDT\nReceiving address: {address}\nExpires: {expires_at}\n\n⚠️ Send exactly {amount_usdt} USDT on BNB Smart Chain (BEP20). Do not round, underpay, or overpay. Wrong amount or wrong network requires manual review and may delay wallet credit.",
        &[
            ("amount_vnd", format_vnd(payment.amount_vnd)),
            ("amount_usdt", copyable_code(&amount)),
            ("address", copyable_code(address)),
            ("expires_at", payment.expires_at.clone()),
        ],
    );
    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(wallet_crypto_topup_keyboard(ctx, lang, payment.id, true))
        .await?;
    Ok(())
}

fn wallet_back_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Back"),
        "start:wallet",
    )]])
}

fn wallet_crypto_topup_keyboard(
    ctx: &AppContext,
    lang: &str,
    payment_id: i64,
    allow_check: bool,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if allow_check {
        rows.push(vec![
            InlineKeyboardButton::callback(
                i18n::t(ctx, lang, "check_crypto_btn", "Check payment"),
                format!("wallettopupcheck:{payment_id}"),
            ),
            InlineKeyboardButton::callback(
                i18n::t(ctx, lang, "cancel_crypto_btn", "Cancel USDT"),
                format!("wallettopupcancel:{payment_id}"),
            ),
        ]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "cancel_crypto_btn", "Cancel USDT"),
            format!("wallettopupcancel:{payment_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Back"),
        "start:wallet",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn topup_cancelled_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                i18n::t(ctx, lang, "wallet_btn_topup_again", "💰 Top up again"),
                "wallet:topup",
            ),
            InlineKeyboardButton::callback(
                i18n::t(ctx, lang, "wallet_btn_back", "⬅️ Back"),
                "start:wallet",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "start_btn_shop", "🛒 Shop"),
            "start:shop",
        )],
    ])
}

async fn send_binance_topup_instructions(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    payment: &CryptoPaymentRequest,
    lang: &str,
) -> Result<()> {
    let amount = bep20_pay::format_bep20_amount(payment.amount_usdt_expected);
    let pay_id = ctx.binance_pay_receiver_pay_id().unwrap_or_default();
    let receiver_name = ctx.binance_pay_receiver_name().unwrap_or_default();
    let text = i18n::tr(
        ctx,
        lang,
        "topup_binance_note_instructions",
        "🟡 NẠP USDT QUA BINANCE PAY\n\n━━━━━━━━━━━━━━━━━━━\n💵 Số USDT cần chuyển: {amount_usdt} USDT\n💰 Quy đổi: ≈ {amount_vnd}\n━━━━━━━━━━━━━━━━━━━\n\n📲 Thông tin Binance Pay:\n• Pay ID: {pay_id}\n• Binance ID: {receiver_name}\n\n📝 Nội dung ghi chú: {memo}\n\n⚠️ QUAN TRỌNG: Ghi chính xác mã nội dung để hệ thống nhận biết!\n⏱️ Hệ thống tự động xác nhận sau 1-2 phút.\n\n⚠️ Vui lòng không hủy đơn trong thời gian thanh toán.\nNếu vượt quá thời gian không thấy + tiền vui lòng liên hệ admin.\n⏳ Hết hạn: {expires_at}",
        &[
            ("amount_vnd", format_vnd(payment.amount_vnd)),
            ("amount_usdt", copyable_code(&amount)),
            ("pay_id", copyable_code(&pay_id)),
            ("receiver_name", html_escape(&receiver_name)),
            ("memo", copyable_code(&payment.memo)),
            ("expires_at", payment.expires_at.clone()),
        ],
    );
    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(wallet_crypto_topup_keyboard(ctx, lang, payment.id, true))
        .await?;
    Ok(())
}

async fn handle_binance_topup_check(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    payment_id: &str,
) -> Result<()> {
    handle_wallet_crypto_topup_check(ctx, chat_id, user_id, payment_id, true).await
}

async fn handle_wallet_crypto_topup_check(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    payment_id: &str,
    require_binance: bool,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    let Some(payment) = wallet_crypto_payment_for_user(ctx, payment_id, user_id, chat_id.0).await?
    else {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_payment_not_found",
                    "Payment request not found.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    };
    if payment.purpose != "wallet_topup"
        || (require_binance && payment.method != CryptoPaymentMethod::BinancePay)
    {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_action_invalid",
                    "Invalid payment action.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }
    if !matches!(
        payment.status,
        CryptoPaymentStatus::Pending | CryptoPaymentStatus::Confirming
    ) {
        ctx.bot
            .send_message(
                chat_id,
                wallet_crypto_status_message(ctx, &lang, &payment, true),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    let refreshed = if payment.method == CryptoPaymentMethod::BinancePay {
        if let Err(err) = binance_worker::run_binance_pay_tick(ctx.clone()).await {
            warn!(
                "on-demand Binance Pay note scan failed for {}: {err}",
                payment.id
            );
        }
        crypto_repo::find_crypto_payment_by_id(&ctx.pool, payment.id)
            .await?
            .unwrap_or(payment)
    } else {
        payment
    };
    let message = if refreshed.status == CryptoPaymentStatus::Completed {
        i18n::tr(
            ctx,
            &lang,
            "topup_binance_completed",
            "Payment completed. Wallet credited: {amount}.",
            &[("amount", format_vnd(refreshed.amount_vnd))],
        )
    } else if refreshed.method == CryptoPaymentMethod::BinancePay {
        i18n::tr(
            ctx,
            &lang,
            "topup_binance_check_not_found",
            "Hệ thống chưa thấy giao dịch Binance Pay khớp mã {memo}.\nVui lòng kiểm tra đã nhập đúng ghi chú và đúng số USDT.",
            &[("memo", refreshed.memo.clone())],
        )
    } else {
        wallet_crypto_status_message(ctx, &lang, &refreshed, true)
    };
    ctx.bot
        .send_message(chat_id, message)
        .reply_markup(wallet_back_keyboard(ctx, &lang))
        .await?;
    Ok(())
}

async fn handle_binance_topup_cancel(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    payment_id: &str,
) -> Result<()> {
    handle_wallet_crypto_topup_cancel(ctx, chat_id, user_id, payment_id, true).await
}

async fn handle_wallet_crypto_topup_cancel(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    user_id: i64,
    payment_id: &str,
    require_binance: bool,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    let Some(payment) = wallet_crypto_payment_for_user(ctx, payment_id, user_id, chat_id.0).await?
    else {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_payment_not_found",
                    "Payment request not found.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    };
    if payment.purpose != "wallet_topup"
        || (require_binance && payment.method != CryptoPaymentMethod::BinancePay)
        || !matches!(payment.status, CryptoPaymentStatus::Pending)
    {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_cancel_not_pending",
                    "Only pending payment requests can be cancelled.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }
    let cancelled = crypto_repo::expire_crypto_payment(&ctx.pool, payment.id).await?;
    ctx.bot
        .send_message(
            chat_id,
            if cancelled {
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_cancelled",
                    "USDT payment request has been cancelled.",
                )
            } else {
                i18n::t(
                    ctx,
                    &lang,
                    "crypto_cancel_not_pending",
                    "Only pending payment requests can be cancelled.",
                )
            },
        )
        .reply_markup(topup_cancelled_keyboard(ctx, &lang))
        .await?;
    Ok(())
}

async fn wallet_crypto_payment_for_user(
    ctx: &AppContext,
    payment_id: &str,
    user_id: i64,
    chat_id: i64,
) -> Result<Option<CryptoPaymentRequest>> {
    let Ok(payment_id) = payment_id.parse::<i64>() else {
        return Ok(None);
    };
    let Some(payment) = crypto_repo::find_crypto_payment_by_id(&ctx.pool, payment_id).await? else {
        return Ok(None);
    };
    if payment.user_id == user_id && payment.chat_id == chat_id {
        Ok(Some(payment))
    } else {
        Ok(None)
    }
}

fn wallet_crypto_status_message(
    ctx: &AppContext,
    lang: &str,
    payment: &CryptoPaymentRequest,
    is_wallet_topup: bool,
) -> String {
    match payment.status {
        CryptoPaymentStatus::Pending => i18n::t(
            ctx,
            lang,
            "crypto_status_pending",
            "Payment is still pending. I will update it automatically after the transfer is detected.",
        ),
        CryptoPaymentStatus::Confirming => i18n::tr(
            ctx,
            lang,
            "crypto_status_confirming",
            "Transfer detected and waiting for confirmations: {confirmations}.",
            &[("confirmations", payment.confirmations.to_string())],
        ),
        CryptoPaymentStatus::Completed if is_wallet_topup => i18n::t(
            ctx,
            lang,
            "topup_crypto_status_completed",
            "Payment completed. Your wallet has been credited.",
        ),
        CryptoPaymentStatus::Completed => {
            i18n::t(ctx, lang, "crypto_status_completed", "Payment completed.")
        }
        CryptoPaymentStatus::Expired => i18n::t(
            ctx,
            lang,
            "crypto_status_expired",
            "This payment request has expired. Create a new USDT payment if you still want to pay.",
        ),
        CryptoPaymentStatus::Failed => payment.failure_reason.clone().unwrap_or_else(|| {
            i18n::t(
                ctx,
                lang,
                "crypto_status_failed",
                "Payment failed. Please contact support.",
            )
        }),
        CryptoPaymentStatus::ManualReview => i18n::t(
            ctx,
            lang,
            "crypto_status_manual_review",
            "Payment is under manual review. Please wait for admin support.",
        ),
    }
}

pub async fn send_topup_qr(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    memo: &str,
    amount: i64,
    topup_id: i64,
    created_at: &str,
    lang: &str,
) -> Result<()> {
    let qr_url: Url = vietqr_link(&ctx.bank_name(), &ctx.bank_account(), amount, memo).parse()?;

    let bank_holder = ctx
        .bank_account_name()
        .map(|n| format!(" — {}", html_escape(&n)))
        .unwrap_or_default();
    let bank_name = html_escape(&ctx.bank_name());

    let base_caption = i18n::tr(
        ctx,
        lang,
        "topup_qr_caption",
        "💰 TOP-UP REQUEST\n\n\
Amount: {amount}\n\n\
─────────────────────\n\
💳 Account: {acct}\n\
🏦 Bank: {bank}{holder}\n\
📝 TRANSFER MEMO: {memo}\n\
─────────────────────\n\n\
⚠️ Enter the exact memo so the system can process automatically.\n\
⏳ This request expires after {ttl} minutes.",
        &[
            ("amount", format_vnd(amount)),
            ("acct", copyable_code(&ctx.bank_account())),
            ("bank", bank_name),
            ("holder", bank_holder),
            ("memo", copyable_code(memo)),
            ("ttl", TOPUP_TTL_MINUTES.to_string()),
        ],
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "start_btn_shop", "🛒 Shop"),
            "start:shop",
        ),
        InlineKeyboardButton::callback(
            i18n::t(ctx, lang, "topup_btn_cancel", "❌ Cancel request"),
            format!("canceltopup:{}", topup_id),
        ),
    ]]);

    let expires_at = topup_expires_at(created_at);
    let initial_caption = render_qr_caption(
        &base_caption,
        &i18n::tr(
            ctx,
            lang,
            "qr_countdown",
            "⏰ QR valid for: {time}",
            &[("time", mmss((expires_at - Utc::now()).num_seconds()))],
        ),
    );
    let qr_message = ctx
        .bot
        .send_photo(chat_id, InputFile::url(qr_url))
        .caption(initial_caption)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard.clone())
        .await?;
    spawn_topup_qr_countdown(
        ctx.clone(),
        chat_id,
        qr_message.id,
        topup_id,
        base_caption,
        expires_at,
        keyboard,
        lang.to_string(),
    );

    Ok(())
}

// ──────────────────────────────────────────────
// Hủy yêu cầu nạp tiền
// ──────────────────────────────────────────────

async fn handle_cancel_topup(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    msg_id: MessageId,
    user_id: i64,
    topup_id: i64,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(ctx, user_id).await;
    let topup = sqlx::query_as::<_, crate::domains::wallet::models::WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests WHERE id = ?",
    )
    .bind(topup_id)
    .fetch_optional(&ctx.pool)
    .await?;

    let Some(topup) = topup else {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_not_found",
                    "⚠️ Top-up request was not found.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    };

    if topup.user_id != user_id {
        ctx.bot
            .send_message(
                chat_id,
                i18n::t(
                    ctx,
                    &lang,
                    "topup_cancel_not_owner",
                    "⚠️ You cannot cancel this request.",
                ),
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    if topup.status != "pending" {
        ctx.bot
            .send_message(
                chat_id,
                if topup.status == "completed" {
                    i18n::t(
                        ctx,
                        &lang,
                        "topup_already_completed",
                        "⚠️ This request has already been processed and cannot be cancelled.",
                    )
                } else {
                    i18n::t(
                        ctx,
                        &lang,
                        "topup_already_expired",
                        "⚠️ This request has expired.",
                    )
                },
            )
            .reply_markup(wallet_back_keyboard(ctx, &lang))
            .await?;
        return Ok(());
    }

    wallet_repo::expire_topup(&ctx.pool, topup_id).await?;

    let done_text = i18n::t(
        ctx,
        &lang,
        "topup_cancelled",
        "✅ Top-up request has been cancelled.",
    );
    let keyboard = topup_cancelled_keyboard(ctx, &lang);

    let edit_result = ctx
        .bot
        .edit_message_caption(chat_id, msg_id)
        .caption(done_text.clone())
        .reply_markup(keyboard.clone())
        .await;

    if edit_result.is_err() {
        let _ = ctx
            .bot
            .edit_message_text(chat_id, msg_id, done_text)
            .reply_markup(keyboard)
            .await;
    }

    Ok(())
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

pub fn format_vnd(amount: i64) -> String {
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

pub async fn generate_topup_memo(pool: &crate::db::DbPool) -> Result<String> {
    for _ in 0..5 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_alphanumeric())
            .map(char::from)
            .take(8)
            .collect::<String>()
            .to_uppercase();
        let memo = format!("NAP{suffix}");
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM wallet_topup_requests WHERE memo = ?",
        )
        .bind(&memo)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if exists == 0 {
            return Ok(memo);
        }
    }
    Err(anyhow::anyhow!("Không tạo được memo NAP unique"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use sqlx::sqlite::SqlitePoolOptions;

    fn test_ctx() -> Arc<AppContext> {
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
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    #[test]
    fn topup_expiry_accepts_sqlite_datetime_format() {
        let expires = topup_expires_at("2026-05-13 10:00:00");

        assert_eq!(expires.to_rfc3339(), "2026-05-13T10:30:00+00:00");
    }

    #[tokio::test]
    async fn custom_topup_amount_prompt_uses_force_reply() {
        let ctx = test_ctx();
        let markup = custom_topup_amount_reply_markup(&ctx, "en");
        let json = serde_json::to_value(&markup).unwrap();

        assert_eq!(json["force_reply"], true);
        assert_eq!(json["input_field_placeholder"], "Example: 100000");
    }

    #[test]
    fn topup_config_error_requires_bank_account() {
        assert!(topup_config_error(""));
        assert!(topup_config_error("   "));
        assert!(!topup_config_error("123456789"));
    }

    #[test]
    fn copyable_code_escapes_html_for_telegram_caption() {
        assert_eq!(
            copyable_code("1999<779>&3939"),
            "<code>1999&lt;779&gt;&amp;3939</code>"
        );
    }

    #[test]
    fn countdown_caption_edit_keeps_inline_keyboard() {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("🛒 Mua hàng", "start:shop"),
            InlineKeyboardButton::callback("❌ Hủy yêu cầu", "canceltopup:1"),
        ]]);
        let payload = keep_qr_keyboard_on_caption_edit(
            teloxide::payloads::EditMessageCaption::new(ChatId(1), MessageId(2))
                .caption("caption")
                .parse_mode(ParseMode::Html),
            &keyboard,
        );
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(
            json["reply_markup"],
            serde_json::to_value(&keyboard).unwrap()
        );
    }

    #[tokio::test]
    async fn wallet_keyboard_shows_usdt_topup_when_bep20_runtime_configured() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let ctx = AppContext::new(
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
            std::collections::HashMap::from([
                (
                    "bep20_merchant_wallet".to_string(),
                    "0x0000000000000000000000000000000000000001".to_string(),
                ),
                ("bscscan_api_key".to_string(), "bsc-key".to_string()),
            ]),
            BotTexts::default(),
            vec![],
        );

        let (_, keyboard) = get_wallet_text_and_keyboard(&ctx, 42).await.unwrap();
        let json = serde_json::to_value(&keyboard).unwrap();
        let callbacks = json["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"wallet:topup_usdt"));
    }

    #[tokio::test]
    async fn wallet_keyboard_shows_binance_topup_when_runtime_configured() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let ctx = AppContext::new(
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
            std::collections::HashMap::from([
                ("binance_pay_note_enabled".to_string(), "1".to_string()),
                ("binance_pay_api_key".to_string(), "api-key".to_string()),
                ("binance_pay_api_secret".to_string(), "secret".to_string()),
                (
                    "binance_pay_receiver_pay_id".to_string(),
                    "209378262".to_string(),
                ),
                (
                    "binance_pay_receiver_name".to_string(),
                    "Receiver".to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let (_, keyboard) = get_wallet_text_and_keyboard(&ctx, 42).await.unwrap();
        let json = serde_json::to_value(&keyboard).unwrap();
        let callbacks = json["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"wallet:topup_binance"));
    }

    #[tokio::test]
    async fn topup_amount_keyboard_has_back_to_wallet() {
        let ctx = test_ctx();
        let keyboard = topup_amount_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["callback_data"], "wallet:show");
    }

    #[tokio::test]
    async fn usdt_topup_amount_keyboard_has_back_to_wallet() {
        let ctx = test_ctx();
        let keyboard = usdt_topup_amount_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["callback_data"], "wallet:show");
    }

    #[tokio::test]
    async fn wallet_crypto_topup_keyboard_has_back_to_wallet() {
        let ctx = test_ctx();
        let keyboard = wallet_crypto_topup_keyboard(&ctx, "vi", 9, true);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:wallet"));
    }

    #[tokio::test]
    async fn topup_cancelled_keyboard_has_back_to_wallet() {
        let ctx = test_ctx();
        let keyboard = topup_cancelled_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:wallet"));
        assert!(callbacks.contains(&"start:shop"));
    }
}
