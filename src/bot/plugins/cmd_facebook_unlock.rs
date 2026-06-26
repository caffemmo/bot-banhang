use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::json;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message,
    ParseMode,
};
use uuid::Uuid;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::{BotDialogue, State, chat_ui, i18n};
use crate::domains::wallet::repo as wallet_repo;

const DEFAULT_PLATFORM_FEE_PERCENT: i64 = 10;

#[derive(Debug, Clone, FromRow)]
struct FacebookUnlockCase {
    id: String,
    user_id: i64,
    chat_id: i64,
    username: Option<String>,
    issue: String,
    case_details: String,
    accepted_quote_id: Option<String>,
    worker_user_id: Option<i64>,
    amount: i64,
    status: String,
    created_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct FacebookUnlockQuote {
    id: String,
    case_id: String,
    worker_user_id: i64,
    worker_chat_id: i64,
    worker_username: Option<String>,
    amount: i64,
    note: Option<String>,
    status: String,
    created_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct FacebookUnlockWorkerApplication {
    id: String,
    user_id: i64,
    chat_id: i64,
    username: Option<String>,
    info: String,
}

pub struct FacebookUnlockCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for FacebookUnlockCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdFacebookUnlock"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "unlockfb".to_string(),
            description: "Mở khóa Facebook".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        let lang = if let Some(user) = msg.from() {
            i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            ctx.normalize_language_code(None)
        };

        if text == "/unlockfb" || button_matches(&ctx, &lang, "start_btn_facebook_unlock", text) {
            send_unlock_menu(&ctx, msg.chat.id, &lang).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        let Some(state) = dialogue.get().await? else {
            return Ok(false);
        };

        match state {
            State::FacebookUnlockCustomerUsername => {
                if text.is_empty() {
                    ask_customer_telegram_username(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                let Some(username) = validate_own_telegram_username(&ctx, &msg, text).await? else {
                    ask_customer_telegram_username(&ctx, msg.chat.id).await?;
                    return Ok(true);
                };
                dialogue.update(State::FacebookUnlockIssue).await?;
                ask_issue(&ctx, msg.chat.id).await?;
                let _ = username;
                Ok(true)
            }
            State::FacebookUnlockIssue => {
                if text.is_empty() {
                    ask_issue(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                dialogue
                    .update(State::FacebookUnlockOwnership {
                        issue: text.to_string(),
                    })
                    .await?;
                ask_account_ownership(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockDetails { issue } => {
                if text.is_empty() {
                    ask_account_ownership(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                dialogue
                    .update(State::FacebookUnlockOwnership { issue })
                    .await?;
                ask_account_ownership(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockOwnership { issue } => {
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                ask_account_ownership(&ctx, msg.chat.id).await?;
                dialogue
                    .update(State::FacebookUnlockOwnership { issue })
                    .await?;
                Ok(true)
            }
            State::FacebookUnlockLockedDuration { issue, ownership } => {
                if text.is_empty() {
                    ask_locked_duration(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                dialogue
                    .update(State::FacebookUnlockCaseNote {
                        issue,
                        ownership,
                        locked_duration: text.to_string(),
                        customer_username: telegram_username_from_message(&msg).unwrap_or_default(),
                    })
                    .await?;
                ask_case_note(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockCaseNote {
                issue,
                ownership,
                locked_duration,
                customer_username,
            } => {
                if text.is_empty() {
                    ask_case_note(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                submit_unlock_case(
                    ctx.clone(),
                    &msg,
                    issue,
                    ownership,
                    locked_duration,
                    customer_username,
                    text.to_string(),
                    &lang,
                )
                .await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            State::FacebookUnlockWorkerApply => {
                if text.is_empty() {
                    ask_worker_telegram_username(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                let Some(username) = validate_own_telegram_username(&ctx, &msg, text).await? else {
                    ask_worker_telegram_username(&ctx, msg.chat.id).await?;
                    return Ok(true);
                };
                dialogue
                    .update(State::FacebookUnlockWorkerServices {
                        telegram_username: username,
                    })
                    .await?;
                ask_worker_services(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockWorkerServices { telegram_username } => {
                if text.is_empty() {
                    ask_worker_services(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                dialogue
                    .update(State::FacebookUnlockWorkerRate {
                        telegram_username,
                        services: text.to_string(),
                    })
                    .await?;
                ask_worker_rate(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockWorkerRate {
                telegram_username,
                services,
            } => {
                if text.is_empty() {
                    ask_worker_rate(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                submit_worker_application(ctx.clone(), &msg, telegram_username, services, text.to_string()).await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            State::FacebookUnlockQuote { case_id } => {
                if text.is_empty() {
                    ask_quote_amount(&ctx, msg.chat.id, &case_id).await?;
                    return Ok(true);
                }
                submit_quote(ctx.clone(), &msg, &case_id, text, &lang).await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            State::FacebookUnlockWorkerMessage { case_id } => {
                if text.is_empty() {
                    ask_relay_message(&ctx, msg.chat.id, &case_id, "khách").await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                relay_worker_message(ctx.clone(), &msg, &case_id, text).await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            State::FacebookUnlockCustomerMessage { case_id } => {
                if text.is_empty() {
                    ask_relay_message(&ctx, msg.chat.id, &case_id, "dịch vụ").await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                relay_customer_message(ctx.clone(), &msg, &case_id, text).await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(data) = q.data.as_deref() else {
            return Ok(false);
        };
        if !data.starts_with("fbunlock:") {
            return Ok(false);
        }

        let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };
        let chat_id = msg.chat().id;

        match data {
            "fbunlock:menu" => {
                send_unlock_menu(&ctx, chat_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:customer" => {
                ask_customer_telegram_username(&ctx, chat_id).await?;
                dialogue.update(State::FacebookUnlockCustomerUsername).await?;
            }
            "fbunlock:customer_my_cases" => {
                send_customer_my_cases(&ctx, chat_id, q.from.id.0 as i64, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:customer_delete_menu" => {
                send_customer_case_action_menu(&ctx, chat_id, q.from.id.0 as i64, &lang, "delete")
                    .await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:customer_cancel_menu" => {
                send_customer_case_action_menu(&ctx, chat_id, q.from.id.0 as i64, &lang, "cancel")
                    .await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:customer_message_menu" => {
                send_customer_case_action_menu(&ctx, chat_id, q.from.id.0 as i64, &lang, "message")
                    .await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:worker" => {
                send_worker_menu(&ctx, chat_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:worker_apply" => {
                ask_worker_telegram_username(&ctx, chat_id).await?;
                dialogue.update(State::FacebookUnlockWorkerApply).await?;
            }
            "fbunlock:worker_cases" => {
                send_worker_case_list(&ctx, chat_id, q.from.id.0 as i64, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:worker_my_cases" => {
                send_worker_my_cases(&ctx, chat_id, q.from.id.0 as i64, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:owned_yes" | "fbunlock:owned_no" => {
                let Some(State::FacebookUnlockOwnership { issue }) = dialogue.get().await? else {
                    ask_issue(&ctx, chat_id).await?;
                    dialogue.update(State::FacebookUnlockIssue).await?;
                    return Ok(true);
                };
                let ownership = if data == "fbunlock:owned_yes" {
                    "Có"
                } else {
                    "Không"
                }
                .to_string();
                if let Some(message) = &q.message {
                    let _ = ctx
                        .bot
                        .edit_message_text(
                            chat_id,
                            message.id(),
                            format!("Tài khoản của bạn có chính chủ không?\n\nĐã chọn: {ownership}"),
                        )
                        .await;
                }
                ask_locked_duration(&ctx, chat_id).await?;
                dialogue
                    .update(State::FacebookUnlockLockedDuration { issue, ownership })
                    .await?;
            }
            _ if data.starts_with("fbunlock:approve_worker:") => {
                let application_id = data.trim_start_matches("fbunlock:approve_worker:");
                approve_worker_application(ctx.clone(), chat_id, q.from.id.0 as i64, application_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:reject_worker:") => {
                let application_id = data.trim_start_matches("fbunlock:reject_worker:");
                reject_worker_application(ctx.clone(), chat_id, q.from.id.0 as i64, application_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:quote:") => {
                let case_id = data.trim_start_matches("fbunlock:quote:").to_string();
                ask_quote_amount(&ctx, chat_id, &case_id).await?;
                dialogue.update(State::FacebookUnlockQuote { case_id }).await?;
            }
            _ if data.starts_with("fbunlock:accept_quote:") => {
                let quote_id = data.trim_start_matches("fbunlock:accept_quote:");
                accept_quote(ctx.clone(), chat_id, q.from.id.0 as i64, quote_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:pay_quote:") => {
                let quote_id = data.trim_start_matches("fbunlock:pay_quote:");
                pay_accepted_quote(ctx.clone(), chat_id, q.from.id.0 as i64, quote_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:msg_customer:") => {
                let case_id = data.trim_start_matches("fbunlock:msg_customer:").to_string();
                ask_relay_message(&ctx, chat_id, &case_id, "khách").await?;
                dialogue.update(State::FacebookUnlockWorkerMessage { case_id }).await?;
            }
            _ if data.starts_with("fbunlock:msg_worker:") => {
                let case_id = data.trim_start_matches("fbunlock:msg_worker:").to_string();
                ask_relay_message(&ctx, chat_id, &case_id, "dịch vụ").await?;
                dialogue.update(State::FacebookUnlockCustomerMessage { case_id }).await?;
            }
            _ if data.starts_with("fbunlock:worker_done:") => {
                let case_id = data.trim_start_matches("fbunlock:worker_done:");
                worker_mark_done(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:worker_failed:") => {
                let case_id = data.trim_start_matches("fbunlock:worker_failed:");
                worker_mark_failed(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:cancel_case:") => {
                let case_id = data.trim_start_matches("fbunlock:cancel_case:");
                request_cancel_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:confirm_delete_case:") => {
                let case_id = data.trim_start_matches("fbunlock:confirm_delete_case:");
                customer_delete_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:delete_case:") => {
                let case_id = data.trim_start_matches("fbunlock:delete_case:");
                confirm_customer_delete_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:hide_case:") => {
                let case_id = data.trim_start_matches("fbunlock:hide_case:");
                customer_hide_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:confirm_done:") => {
                let case_id = data.trim_start_matches("fbunlock:confirm_done:");
                complete_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:dispute:") => {
                let case_id = data.trim_start_matches("fbunlock:dispute:");
                dispute_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:admin_refund:") => {
                let case_id = data.trim_start_matches("fbunlock:admin_refund:");
                admin_refund_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:admin_reject_refund:") => {
                let case_id = data.trim_start_matches("fbunlock:admin_reject_refund:");
                admin_reject_refund(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:admin_reopen:") => {
                let case_id = data.trim_start_matches("fbunlock:admin_reopen:");
                admin_reopen_case(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ if data.starts_with("fbunlock:admin_delete_customer_cases:") => {
                let case_id = data.trim_start_matches("fbunlock:admin_delete_customer_cases:");
                admin_delete_customer_cases(ctx.clone(), chat_id, q.from.id.0 as i64, case_id).await?;
                dialogue.update(State::Idle).await?;
            }
            _ => {}
        }

        Ok(true)
    }
}

async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS facebook_unlock_cases (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            username TEXT,
            issue TEXT NOT NULL,
            case_details TEXT,
            account_info TEXT,
            amount INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'open',
            accepted_quote_id TEXT,
            worker_user_id INTEGER,
            worker_note TEXT,
            paid_at TEXT,
            completed_at TEXT,
            refunded_at TEXT,
            payout_at TEXT,
            platform_fee INTEGER NOT NULL DEFAULT 0,
            worker_payout INTEGER NOT NULL DEFAULT 0,
            customer_hidden INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    for column in [
        "case_details TEXT",
        "accepted_quote_id TEXT",
        "worker_user_id INTEGER",
        "worker_note TEXT",
        "paid_at TEXT",
        "completed_at TEXT",
        "refunded_at TEXT",
        "payout_at TEXT",
        "platform_fee INTEGER NOT NULL DEFAULT 0",
        "worker_payout INTEGER NOT NULL DEFAULT 0",
        "customer_hidden INTEGER NOT NULL DEFAULT 0",
    ] {
        let sql = format!("ALTER TABLE facebook_unlock_cases ADD COLUMN {column}");
        let _ = sqlx::query(&sql).execute(pool).await;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS facebook_unlock_quotes (
            id TEXT PRIMARY KEY,
            case_id TEXT NOT NULL,
            worker_user_id INTEGER NOT NULL,
            worker_chat_id INTEGER NOT NULL,
            worker_username TEXT,
            amount INTEGER NOT NULL,
            note TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS facebook_unlock_workers (
            user_id INTEGER PRIMARY KEY,
            chat_id INTEGER NOT NULL,
            username TEXT,
            info TEXT,
            status TEXT NOT NULL DEFAULT 'approved',
            approved_by INTEGER,
            approved_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS facebook_unlock_worker_applications (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            username TEXT,
            info TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS facebook_unlock_messages (
            id TEXT PRIMARY KEY,
            case_id TEXT NOT NULL,
            sender_role TEXT NOT NULL,
            sender_user_id INTEGER NOT NULL,
            message TEXT NOT NULL,
            created_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

fn platform_fee_percent(ctx: &AppContext) -> i64 {
    ctx.get_text(
        "facebook_unlock_platform_fee_percent",
        &DEFAULT_PLATFORM_FEE_PERCENT.to_string(),
    )
    .trim()
    .parse::<i64>()
    .ok()
    .filter(|percent| (0..=100).contains(percent))
    .unwrap_or(DEFAULT_PLATFORM_FEE_PERCENT)
}

fn button_matches(ctx: &AppContext, lang: &str, key: &str, text: &str) -> bool {
    i18n::button_text_match_variants(&i18n::t(ctx, lang, key, "🔓 Mở khóa Facebook"))
        .iter()
        .any(|variant| variant.eq_ignore_ascii_case(text))
}

async fn send_unlock_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let text = "🔓 <b>MỞ KHÓA FACEBOOK</b>\n\n\
        Bot là trung gian giữa khách và người làm dịch vụ.\n\n\
        • Khách tạo case miễn phí để nhận báo giá.\n\
        • Nhiều người dịch vụ có thể cùng báo giá một case.\n\
        • Khách chọn giá phù hợp rồi thanh toán cho bot giữ tiền trung gian.\n\
        • Khi case đã thanh toán, người dịch vụ mới nhận case để xử lý.";
    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": {
                "inline_keyboard": [
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_customer", "🙋 Tôi cần mở khóa Facebook", "fbunlock:customer")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_customer_my_cases", "🧾 Case của tôi", "fbunlock:customer_my_cases")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_worker", "🧑‍💻 Tôi là người dịch vụ", "fbunlock:worker")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_back", "⬅️ Quay lại", "start:menu")]
                ]
            }
        }),
    )
    .await?;
    Ok(())
}

async fn send_worker_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let fee_percent = platform_fee_percent(ctx);
    let text = format!(
        "🧑‍💻 <b>KHU NGƯỜI LÀM DỊCH VỤ</b>\n\n\
         Bạn có thể xem các case đang chờ báo giá và gửi giá xử lý cho khách.\n\n\
         Phí sàn nội bộ: <b>{fee_percent}%</b> trên case thành công.\n\
         Ví dụ báo giá 300.000đ, phí sàn {fee_percent}%, bạn nhận dự kiến {} sau khi hoàn tất.",
        format_vnd(300_000 - (300_000 * fee_percent / 100))
    );
    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": {
                "inline_keyboard": [
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_worker_cases", "📋 Xem case cần báo giá", "fbunlock:worker_cases")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_worker_my_cases", "🧾 Case của tôi", "fbunlock:worker_my_cases")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_worker_apply", "📝 Đăng ký làm dịch vụ", "fbunlock:worker_apply")],
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_back", "⬅️ Quay lại", "fbunlock:menu")]
                ]
            }
        }),
    )
    .await?;
    Ok(())
}

async fn ask_issue(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "📌 Tài khoản Facebook của bạn đang bị vấn đề gì?\n\nVí dụ: checkpoint, khóa 956, két sắt, xác minh danh tính, mất 2FA...",
        )
        .await?;
    Ok(())
}

async fn ask_customer_telegram_username(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "Vui lòng nhập user Telegram của bạn:\nVD: @tencuaban\n\nLưu ý: phải đúng username của tài khoản Telegram đang dùng bot.",
        )
        .await?;
    Ok(())
}

async fn ask_account_ownership(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "Tài khoản của bạn có chính chủ không?",
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("Có", "fbunlock:owned_yes"),
            InlineKeyboardButton::callback("Không", "fbunlock:owned_no"),
        ]]))
        .await?;
    Ok(())
}

async fn ask_locked_duration(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(chat_id, "Tài khoản bạn bị khóa bao nhiêu lâu?")
        .await?;
    Ok(())
}

async fn ask_case_note(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "Bạn muốn mô tả tiêu đề cho CASE của bạn cho dịch vụ xem như nào?\n\nVD: acc em bị khóa 956, nhưng mà có đầy đủ giấy tờ, ai mở được thì nhận case nha!",
        )
        .await?;
    Ok(())
}

async fn ask_worker_telegram_username(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "Vui lòng nhập user Telegram của bạn:\nVD: @tencuaban\n\nLưu ý: phải đúng username của tài khoản Telegram đang dùng bot.",
        )
        .await?;
    Ok(())
}

async fn ask_worker_services(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(chat_id, "Dịch vụ xử lý được:\nVD: 282, 956, FAQ")
        .await?;
    Ok(())
}

async fn ask_worker_rate(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(chat_id, "Tỉ lệ nhận case:\nVD: 100%")
        .await?;
    Ok(())
}

async fn ask_quote_amount(ctx: &AppContext, chat_id: ChatId, case_id: &str) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "💬 Nhập báo giá cho case <code>{}</code>.\n\nVí dụ: <code>300000</code> hoặc <code>300000 | Có thể xử lý trong 24h</code>",
                html_escape(case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn submit_unlock_case(
    ctx: Arc<AppContext>,
    msg: &Message,
    issue: String,
    ownership: String,
    locked_duration: String,
    customer_username: String,
    case_note: String,
    lang: &str,
) -> Result<()> {
    let Some(user) = msg.from() else {
        ctx.bot
            .send_message(msg.chat.id, i18n::t(&ctx, lang, "user_unknown", "Cannot identify user."))
            .await?;
        return Ok(());
    };

    let case_id = format!("FBUNLOCK-{}", short_id());
    let now = Utc::now().to_rfc3339();
    let case_details = format!(
        "Telegram khách: {}\nTài khoản chính chủ: {}\nThời gian bị khóa: {}\nThông tin khách note case: {}",
        customer_username.trim(),
        ownership.trim(),
        locked_duration.trim(),
        case_note.trim()
    );
    sqlx::query(
        r#"
        INSERT INTO facebook_unlock_cases
        (id, user_id, chat_id, username, issue, case_details, account_info, amount, status, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, 0, 'open', ?, ?)
        "#,
    )
    .bind(&case_id)
    .bind(user.id.0 as i64)
    .bind(msg.chat.id.0)
    .bind(user.username.clone())
    .bind(issue.trim())
    .bind(case_details.as_str())
    .bind(case_details.as_str())
    .bind(&now)
    .bind(&now)
    .execute(&ctx.pool)
    .await?;

    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã tạo case mở khóa Facebook.\n\nMã case: <code>{}</code>\nTrạng thái: chờ người dịch vụ báo giá.\n\nBạn chưa bị trừ tiền. Khi có báo giá, bot sẽ gửi để bạn chọn và thanh toán.",
                case_id
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(case_created_keyboard(&ctx, lang))
        .await?;

    if let Some(case) = load_case(&ctx.pool, &case_id).await? {
        notify_admins_new_case(&ctx, &case).await;
        notify_workers_new_case(&ctx, &case).await;
    }

    Ok(())
}

async fn submit_worker_application(
    ctx: Arc<AppContext>,
    msg: &Message,
    telegram_username: String,
    services: String,
    receive_rate: String,
) -> Result<()> {
    let Some(user) = msg.from() else {
        ctx.bot.send_message(msg.chat.id, "Không xác định được user.").await?;
        return Ok(());
    };

    let application_id = format!("FBWORKER-{}", short_id());
    let now = Utc::now().to_rfc3339();
    let info = format!(
        "Telegram dịch vụ: {}\nDịch vụ xử lý được: {}\nTỉ lệ nhận case: {}",
        telegram_username.trim(),
        services.trim(),
        receive_rate.trim()
    );
    sqlx::query(
        r#"
        INSERT INTO facebook_unlock_worker_applications
        (id, user_id, chat_id, username, info, status, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)
        "#,
    )
    .bind(&application_id)
    .bind(user.id.0 as i64)
    .bind(msg.chat.id.0)
    .bind(user.username.clone())
    .bind(info.as_str())
    .bind(&now)
    .bind(&now)
    .execute(&ctx.pool)
    .await?;

    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã nhận đăng ký làm dịch vụ.\nMã đăng ký: <code>{}</code>\n\nAdmin sẽ kiểm tra và liên hệ khi cần.",
                application_id
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;

    notify_admins_worker_application(&ctx, &application_id, user.id.0 as i64, user.username.as_deref(), &info).await;
    Ok(())
}

async fn approve_worker_application(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    application_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền duyệt worker.").await?;
        return Ok(());
    }
    let Some(application) = load_worker_application(&ctx.pool, application_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy đăng ký worker.").await?;
        return Ok(());
    };
    let now = Utc::now().to_rfc3339();
    let mut tx = ctx.pool.begin().await?;
    sqlx::query(
        "UPDATE facebook_unlock_worker_applications SET status = 'approved', updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(application_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO facebook_unlock_workers
         (user_id, chat_id, username, info, status, approved_by, approved_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'approved', ?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET chat_id = excluded.chat_id, username = excluded.username,
             info = excluded.info, status = 'approved', approved_by = excluded.approved_by,
             approved_at = excluded.approved_at, updated_at = excluded.updated_at",
    )
    .bind(application.user_id)
    .bind(application.chat_id)
    .bind(application.username.clone())
    .bind(application.info.clone())
    .bind(admin_user_id)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    ctx.bot
        .send_message(chat_id, format!("Đã duyệt worker cho đăng ký <code>{}</code>.", html_escape(application_id)))
        .parse_mode(ParseMode::Html)
        .await?;
    let _ = ctx
        .bot
        .send_message(
            ChatId(application.chat_id),
            "✅ Admin đã duyệt bạn làm dịch vụ mở khóa Facebook. Bạn có thể xem case và báo giá.",
        )
        .await;
    Ok(())
}

async fn reject_worker_application(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    application_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền từ chối worker.").await?;
        return Ok(());
    }
    let Some(application) = load_worker_application(&ctx.pool, application_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy đăng ký worker.").await?;
        return Ok(());
    };
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE facebook_unlock_worker_applications SET status = 'rejected', updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(application_id)
    .execute(&ctx.pool)
    .await?;
    ctx.bot
        .send_message(chat_id, format!("Đã từ chối đăng ký <code>{}</code>.", html_escape(application_id)))
        .parse_mode(ParseMode::Html)
        .await?;
    let _ = ctx
        .bot
        .send_message(
            ChatId(application.chat_id),
            "Admin đã từ chối đăng ký làm dịch vụ mở khóa Facebook.",
        )
        .await;
    Ok(())
}

async fn submit_quote(
    ctx: Arc<AppContext>,
    msg: &Message,
    case_id: &str,
    raw_text: &str,
    lang: &str,
) -> Result<()> {
    let Some(user) = msg.from() else {
        ctx.bot
            .send_message(msg.chat.id, i18n::t(&ctx, lang, "user_unknown", "Cannot identify user."))
            .await?;
        return Ok(());
    };
    if !is_approved_worker(&ctx.pool, user.id.0 as i64).await? {
        ctx.bot
            .send_message(msg.chat.id, "Bạn cần được admin duyệt làm dịch vụ trước khi báo giá.")
            .await?;
        return Ok(());
    }
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(msg.chat.id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.status != "open" {
        ctx.bot
            .send_message(msg.chat.id, "Case này không còn nhận báo giá.")
            .await?;
        return Ok(());
    }

    let Some((amount, note)) = parse_quote(raw_text) else {
        ask_quote_amount(&ctx, msg.chat.id, case_id).await?;
        return Ok(());
    };

    let quote_id = format!("FBQUOTE-{}", short_id());
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT INTO facebook_unlock_quotes
        (id, case_id, worker_user_id, worker_chat_id, worker_username, amount, note, status, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)
        "#,
    )
    .bind(&quote_id)
    .bind(case_id)
    .bind(user.id.0 as i64)
    .bind(msg.chat.id.0)
    .bind(user.username.clone())
    .bind(amount)
    .bind(note.as_deref())
    .bind(&now)
    .bind(&now)
    .execute(&ctx.pool)
    .await?;

    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã gửi báo giá cho case <code>{}</code>.\nGiá: <b>{}</b>\nBot sẽ gửi báo giá này cho khách chọn.",
                html_escape(case_id),
                format_vnd(amount)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;

    if let Some(quote) = load_quote(&ctx.pool, &quote_id).await? {
        notify_customer_quote(&ctx, &case, &quote).await?;
        notify_admins_quote(&ctx, &case, &quote).await;
    }

    Ok(())
}

async fn send_worker_case_list(
    ctx: &AppContext,
    chat_id: ChatId,
    worker_user_id: i64,
    lang: &str,
) -> Result<()> {
    if !is_approved_worker(&ctx.pool, worker_user_id).await? {
        ctx.bot
            .send_message(
                chat_id,
                "Bạn cần đăng ký và được admin duyệt trước khi xem danh sách case.",
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_worker_apply",
                "📝 Đăng ký làm dịch vụ",
                "fbunlock:worker_apply",
            )]]))
            .await?;
        return Ok(());
    }

    let cases = list_open_cases(&ctx.pool, 8).await?;
    if cases.is_empty() {
        ctx.bot
            .send_message(chat_id, "Hiện chưa có case nào đang chờ báo giá.")
            .await?;
        return Ok(());
    }

    let mut lines = vec!["🔓 <b>CASE CHỜ DỊCH VỤ</b>".to_string()];
    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let number = index + 1;
        let note = html_escape(&case_note_info(&case.case_details));
        lines.push(format!(
            "\n╭─ 🧾 #{} · <code>{}</code>\n├ 🔒 Vấn đề: {}\n├ 📝 Ghi chú: <i>{}</i>\n├ 🕓 Thời gian: {}\n╰────────────────",
            number,
            html_escape(&case.id),
            html_escape(&case.issue),
            note,
            html_escape(&format_short_time(&case.created_at))
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            format!("💬 Báo giá #{}", number),
            format!("fbunlock:quote:{}", case.id),
        )]);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "fbunlock_btn_back",
        "⬅️ Quay lại",
        "fbunlock:worker",
    )]);

    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": lines.join("\n"),
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": rows }
        }),
    )
    .await?;
    Ok(())
}

async fn send_worker_my_cases(
    ctx: &AppContext,
    chat_id: ChatId,
    worker_user_id: i64,
    lang: &str,
) -> Result<()> {
    if !is_approved_worker(&ctx.pool, worker_user_id).await? {
        ctx.bot
            .send_message(
                chat_id,
                "Bạn cần đăng ký và được admin duyệt trước khi xem case của tôi.",
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_worker_apply",
                "📝 Đăng ký làm dịch vụ",
                "fbunlock:worker_apply",
            )]]))
            .await?;
        return Ok(());
    }

    let cases = list_worker_in_progress_cases(&ctx.pool, worker_user_id, chat_id.0, 10).await?;
    if cases.is_empty() {
        ctx.bot
            .send_message(chat_id, "Bạn chưa có case nào đang xử lý.")
            .await?;
        return Ok(());
    }

    let mut lines = vec!["🧾 <b>CASE CỦA TÔI</b>".to_string()];
    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let number = index + 1;
        lines.push(format!(
            "\n#{} | Mã case: <code>{}</code>\nTrạng thái: {}\nThời gian: {}\n\nFacebook này đang bị vấn đề:\n{}\n\nThông tin khách note case:\n{}",
            number,
            html_escape(&case.id),
            html_escape(&case.status),
            html_escape(&format_time(&case.created_at)),
            html_escape(&case.issue),
            html_escape(&case_note_info(&case.case_details))
        ));
        rows.push(vec![
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_message_customer",
                "💬 Nhắn khách",
                format!("fbunlock:msg_customer:{}", case.id),
            ),
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_worker_done",
                &format!("✅ Báo đã hoàn tất #{}", number),
                format!("fbunlock:worker_done:{}", case.id),
            ),
        ]);
        rows.push(vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_worker_failed",
            &format!("⚠️ Không xử lý được #{}", number),
            format!("fbunlock:worker_failed:{}", case.id),
        )]);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "fbunlock_btn_back",
        "⬅️ Quay lại",
        "fbunlock:worker",
    )]);

    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": lines.join("\n"),
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": rows }
        }),
    )
    .await?;
    Ok(())
}

async fn send_customer_my_cases(
    ctx: &AppContext,
    chat_id: ChatId,
    customer_user_id: i64,
    lang: &str,
) -> Result<()> {
    let cases = list_customer_cases(&ctx.pool, customer_user_id, chat_id.0, 10).await?;
    if cases.is_empty() {
        ctx.bot
            .send_message(chat_id, "Bạn chưa có case mở khóa Facebook nào.")
            .await?;
        return Ok(());
    }

    let mut lines = vec!["🧾 <b>CASE CỦA TÔI</b>".to_string()];
    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let number = index + 1;
        lines.push(format!(
            "\n╭─ 🧾 #{} · <code>{}</code>\n├ 📌 Trạng thái: <b>{}</b>\n├ 🔒 Vấn đề: {}\n├ 📝 Ghi chú: <i>{}</i>\n├ 🕓 Thời gian: {}\n╰────────────────",
            number,
            html_escape(&case.id),
            html_escape(customer_status_label(&case.status)),
            html_escape(&case.issue),
            html_escape(&case_note_info(&case.case_details)),
            html_escape(&format_short_time(&case.created_at))
        ));
    }
    rows.push(vec![
        InlineKeyboardButton::callback("🗑 Xóa case", "fbunlock:customer_delete_menu"),
        InlineKeyboardButton::callback("❌ Yêu cầu hủy", "fbunlock:customer_cancel_menu"),
    ]);
    rows.push(vec![InlineKeyboardButton::callback(
        "💬 Nhắn tin",
        "fbunlock:customer_message_menu",
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "fbunlock_btn_back",
        "⬅️ Quay lại",
        "fbunlock:menu",
    )]);

    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": lines.join("\n"),
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": rows }
        }),
    )
    .await?;
    Ok(())
}

async fn send_customer_case_action_menu(
    ctx: &AppContext,
    chat_id: ChatId,
    customer_user_id: i64,
    lang: &str,
    action: &str,
) -> Result<()> {
    let cases = list_customer_cases(&ctx.pool, customer_user_id, chat_id.0, 10).await?;
    let (title, empty_text) = match action {
        "delete" => (
            "🗑 <b>CHỌN CASE MUỐN XÓA/ẨN</b>",
            "Không có case nào có thể xóa hoặc ẩn.",
        ),
        "cancel" => (
            "❌ <b>CHỌN CASE MUỐN YÊU CẦU HỦY</b>",
            "Không có case nào có thể yêu cầu hủy.",
        ),
        "message" => (
            "💬 <b>CHỌN CASE MUỐN NHẮN TIN</b>",
            "Không có case nào đang ở trạng thái có thể nhắn dịch vụ.",
        ),
        _ => ("🧾 <b>CHỌN CASE</b>", "Không có case phù hợp."),
    };

    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let number = index + 1;
        let issue = short_button_text(&case.issue, 24);
        if action == "delete" && case.status == "open" {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("#{} · Xóa · {}", number, issue),
                format!("fbunlock:delete_case:{}", case.id),
            )]);
        } else if action == "delete"
            && matches!(case.status.as_str(), "cancelled" | "completed" | "refunded")
        {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("#{} · Ẩn · {}", number, issue),
                format!("fbunlock:hide_case:{}", case.id),
            )]);
        } else if action == "cancel"
            && matches!(
                case.status.as_str(),
                "quoted_accepted" | "paid_in_progress" | "worker_done" | "worker_failed"
            )
        {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("#{} · Hủy · {}", number, issue),
                format!("fbunlock:cancel_case:{}", case.id),
            )]);
        } else if action == "message"
            && matches!(case.status.as_str(), "paid_in_progress" | "worker_done")
        {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("#{} · Nhắn · {}", number, issue),
                format!("fbunlock:msg_worker:{}", case.id),
            )]);
        }
    }

    let text = if rows.is_empty() {
        format!("{}\n\n{}", title, empty_text)
    } else {
        format!("{}\n\nBấm đúng số case bạn muốn thao tác.", title)
    };
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "fbunlock_btn_back",
        "⬅️ Quay lại",
        "fbunlock:customer_my_cases",
    )]);

    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": rows }
        }),
    )
    .await?;
    Ok(())
}

async fn confirm_customer_delete_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
    lang: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot
            .send_message(chat_id, "Bạn không có quyền xóa case này.")
            .await?;
        return Ok(());
    }
    if case.status != "open" {
        ctx.bot
            .send_message(
                chat_id,
                "Chỉ xóa được case chưa có dịch vụ nhận. Case khác hãy dùng hủy hoặc ẩn.",
            )
            .await?;
        return Ok(());
    }
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "Bạn chắc chắn muốn xóa case <code>{}</code>?\nCase sẽ biến mất khỏi danh sách và không gửi cho dịch vụ nữa.",
                html_escape(case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            i18n::inline_button_callback(
                ctx.as_ref(),
                lang,
                "fbunlock_btn_confirm_delete_case",
                "✅ Xóa",
                format!("fbunlock:confirm_delete_case:{case_id}"),
            ),
            i18n::inline_button_callback(
                ctx.as_ref(),
                lang,
                "fbunlock_btn_back",
                "↩️ Không",
                "fbunlock:customer_my_cases",
            ),
        ]]))
        .await?;
    Ok(())
}

async fn customer_delete_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot
            .send_message(chat_id, "Bạn không có quyền xóa case này.")
            .await?;
        return Ok(());
    }
    if case.status != "open" {
        ctx.bot
            .send_message(chat_id, "Chỉ xóa được case chưa có dịch vụ nhận.")
            .await?;
        return Ok(());
    }

    let mut tx = ctx.pool.begin().await?;
    sqlx::query("DELETE FROM facebook_unlock_messages WHERE case_id = ?")
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM facebook_unlock_quotes WHERE case_id = ?")
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    let deleted = sqlx::query(
        "DELETE FROM facebook_unlock_cases
         WHERE id = ? AND status = 'open' AND (user_id = ? OR chat_id = ?)",
    )
    .bind(case_id)
    .bind(customer_user_id)
    .bind(chat_id.0)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;

    if deleted == 0 {
        ctx.bot
            .send_message(chat_id, "Case này không còn xóa được.")
            .await?;
        return Ok(());
    }
    ctx.bot
        .send_message(
            chat_id,
            format!("Đã xóa case <code>{}</code> khỏi danh sách.", html_escape(case_id)),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn customer_hide_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot
            .send_message(chat_id, "Bạn không có quyền ẩn case này.")
            .await?;
        return Ok(());
    }
    if !matches!(case.status.as_str(), "cancelled" | "completed" | "refunded") {
        ctx.bot
            .send_message(chat_id, "Chỉ ẩn được case đã hủy, hoàn tiền hoặc hoàn tất.")
            .await?;
        return Ok(());
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE facebook_unlock_cases SET customer_hidden = 1, updated_at = ? WHERE id = ?",
    )
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot
        .send_message(
            chat_id,
            format!("Đã ẩn case <code>{}</code> khỏi danh sách của bạn.", html_escape(case_id)),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn accept_quote(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    quote_id: &str,
    lang: &str,
) -> Result<()> {
    let Some(quote) = load_quote(&ctx.pool, quote_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy báo giá.").await?;
        return Ok(());
    };
    let Some(case) = load_case(&ctx.pool, &quote.case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.user_id != user_id {
        ctx.bot.send_message(chat_id, "Bạn không có quyền chọn báo giá này.").await?;
        return Ok(());
    }
    if case.status != "open" {
        ctx.bot.send_message(chat_id, "Case này không còn nhận báo giá.").await?;
        return Ok(());
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE facebook_unlock_quotes SET status = CASE WHEN id = ? THEN 'accepted' ELSE 'rejected' END, updated_at = ?
         WHERE case_id = ?",
    )
    .bind(quote_id)
    .bind(&now)
    .bind(&quote.case_id)
    .execute(&ctx.pool)
    .await?;
    sqlx::query(
        "UPDATE facebook_unlock_cases SET accepted_quote_id = ?, amount = ?, status = 'quoted_accepted', worker_user_id = ?, updated_at = ? WHERE id = ?",
    )
    .bind(quote_id)
    .bind(quote.amount)
    .bind(quote.worker_user_id)
    .bind(&now)
    .bind(&quote.case_id)
    .execute(&ctx.pool)
    .await?;

    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Bạn đã chọn báo giá <b>{}</b> cho case <code>{}</code>.\n\nBấm thanh toán để bot giữ tiền trung gian, sau đó người dịch vụ mới nhận case xử lý.",
                format_vnd(quote.amount),
                html_escape(&quote.case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(pay_quote_keyboard(&ctx, lang, &quote.id, &quote.case_id))
        .await?;

    notify_worker_quote_accepted(&ctx, &quote).await;
    Ok(())
}

async fn pay_accepted_quote(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    quote_id: &str,
    lang: &str,
) -> Result<()> {
    let Some(quote) = load_quote(&ctx.pool, quote_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy báo giá.").await?;
        return Ok(());
    };
    let Some(case) = load_case(&ctx.pool, &quote.case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.user_id != user_id || case.accepted_quote_id.as_deref() != Some(quote_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền thanh toán báo giá này.").await?;
        return Ok(());
    }
    if case.status == "paid_in_progress" {
        ctx.bot.send_message(chat_id, "Case này đã thanh toán rồi.").await?;
        return Ok(());
    }

    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;
    if wallet.balance < quote.amount {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "⚠️ Số dư ví không đủ.\nSố dư hiện tại: {}\nCần thanh toán: {}\n\nVui lòng nạp thêm rồi bấm thanh toán lại.",
                    format_vnd(wallet.balance),
                    format_vnd(quote.amount)
                ),
            )
            .reply_markup(topup_keyboard(&ctx, lang))
            .await?;
        return Ok(());
    }

    let now = Utc::now().to_rfc3339();
    let mut tx = ctx.pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE facebook_unlock_cases
         SET status = 'paid_in_progress', amount = ?, worker_user_id = ?, paid_at = ?, updated_at = ?
         WHERE id = ? AND status = 'quoted_accepted'",
    )
    .bind(quote.amount)
    .bind(quote.worker_user_id)
    .bind(&now)
    .bind(&now)
    .bind(&case.id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        ctx.bot.send_message(chat_id, "Case này không còn ở trạng thái chờ thanh toán.").await?;
        return Ok(());
    }
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        user_id,
        quote.amount,
        &case.id,
        Some("facebook_unlock_escrow"),
    )
    .await?;
    sqlx::query("UPDATE facebook_unlock_quotes SET status = 'paid', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(quote_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Đã thanh toán case <code>{}</code>.\nSố tiền bot đang giữ trung gian: <b>{}</b>\nSố dư còn lại: {}\n\nNgười dịch vụ đã nhận case để xử lý.",
                html_escape(&case.id),
                format_vnd(quote.amount),
                format_vnd(balance_after)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(customer_case_keyboard(&ctx, lang, &case.id))
        .await?;

    notify_worker_case_paid(&ctx, &case, &quote).await;
    notify_admins_case_paid(&ctx, &case, &quote).await;
    Ok(())
}

async fn ask_relay_message(ctx: &AppContext, chat_id: ChatId, case_id: &str, target: &str) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "Nhập nội dung cần nhắn cho {} trong case <code>{}</code>.",
                target,
                html_escape(case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn relay_worker_message(
    ctx: Arc<AppContext>,
    msg: &Message,
    case_id: &str,
    text: &str,
) -> Result<()> {
    let Some(user) = msg.from() else {
        return Ok(());
    };
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(msg.chat.id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !matches!(case.status.as_str(), "paid_in_progress" | "worker_done") {
        ctx.bot
            .send_message(msg.chat.id, "Case này không còn ở trạng thái đang xử lý.")
            .await?;
        return Ok(());
    }
    if !worker_can_handle_case(&ctx.pool, &case, user.id.0 as i64, msg.chat.id.0).await? {
        ctx.bot
            .send_message(msg.chat.id, "Bạn chưa được gắn với case này.")
            .await?;
        return Ok(());
    }
    save_relay_message(&ctx.pool, case_id, "worker", user.id.0 as i64, text).await?;
    ctx.bot
        .send_message(
            ChatId(case.chat_id),
            format!(
                "💬 Tin nhắn từ người dịch vụ cho case <code>{}</code>:\n\n{}",
                html_escape(case_id),
                html_escape(text)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(customer_case_keyboard(&ctx, "vi", case_id))
        .await?;
    ctx.bot.send_message(msg.chat.id, "Đã chuyển tin nhắn cho khách.").await?;
    Ok(())
}

async fn relay_customer_message(
    ctx: Arc<AppContext>,
    msg: &Message,
    case_id: &str,
    text: &str,
) -> Result<()> {
    let Some(user) = msg.from() else {
        return Ok(());
    };
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(msg.chat.id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, user.id.0 as i64, msg.chat.id)
        || !matches!(case.status.as_str(), "paid_in_progress" | "worker_done")
    {
        ctx.bot.send_message(msg.chat.id, "Bạn chưa có quyền nhắn dịch vụ trong case này.").await?;
        return Ok(());
    }
    let Some(quote_id) = case.accepted_quote_id.as_deref() else {
        ctx.bot.send_message(msg.chat.id, "Case chưa có worker được chọn.").await?;
        return Ok(());
    };
    let Some(quote) = load_quote(&ctx.pool, quote_id).await? else {
        ctx.bot.send_message(msg.chat.id, "Không tìm thấy worker của case.").await?;
        return Ok(());
    };
    save_relay_message(&ctx.pool, case_id, "customer", user.id.0 as i64, text).await?;
    ctx.bot
        .send_message(
            ChatId(quote.worker_chat_id),
            format!(
                "💬 Tin nhắn từ khách cho case <code>{}</code>:\n\n{}",
                html_escape(case_id),
                html_escape(text)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(worker_paid_case_keyboard(&ctx, "vi", case_id))
        .await?;
    ctx.bot.send_message(msg.chat.id, "Đã chuyển tin nhắn cho dịch vụ.").await?;
    Ok(())
}

async fn worker_mark_done(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    worker_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.status != "paid_in_progress" {
        ctx.bot
            .send_message(chat_id, "Case này không còn ở trạng thái đang xử lý nên không thể báo hoàn tất.")
            .await?;
        return Ok(());
    }
    if !worker_can_handle_case(&ctx.pool, &case, worker_user_id, chat_id.0).await? {
        ctx.bot.send_message(chat_id, "Bạn chưa được gắn với case này.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE facebook_unlock_cases SET status = 'worker_done', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot.send_message(chat_id, "Đã báo khách xác nhận kết quả.").await?;
    ctx.bot
        .send_message(
            ChatId(case.chat_id),
            format!(
                "✅ Người dịch vụ báo đã hoàn tất case <code>{}</code>. Vui lòng kiểm tra và xác nhận.",
                html_escape(case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(customer_done_keyboard(&ctx, "vi", case_id))
        .await?;
    Ok(())
}

async fn worker_mark_failed(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    worker_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.status != "paid_in_progress" {
        ctx.bot
            .send_message(chat_id, "Case này không còn ở trạng thái đang xử lý nên không thể báo không xử lý được.")
            .await?;
        return Ok(());
    }
    if !worker_can_handle_case(&ctx.pool, &case, worker_user_id, chat_id.0).await? {
        ctx.bot.send_message(chat_id, "Bạn chưa được gắn với case này.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE facebook_unlock_cases SET status = 'worker_failed', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot.send_message(chat_id, "Đã báo admin xử lý case không hoàn tất.").await?;
    notify_admins_worker_failed(&ctx, &case).await;
    Ok(())
}

async fn complete_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot.send_message(chat_id, "Bạn không thể xác nhận case này.").await?;
        return Ok(());
    }
    if !matches!(case.status.as_str(), "paid_in_progress" | "worker_done") {
        ctx.bot
            .send_message(chat_id, "Case này không còn ở trạng thái chờ xác nhận.")
            .await?;
        return Ok(());
    }
    let worker_user_id = if let Some(worker_user_id) = case.worker_user_id {
        worker_user_id
    } else if let Some(quote_id) = case.accepted_quote_id.as_deref() {
        if let Some(quote) = load_quote(&ctx.pool, quote_id).await? {
            quote.worker_user_id
        } else {
            ctx.bot.send_message(chat_id, "Case chưa có worker.").await?;
            return Ok(());
        }
    } else {
        ctx.bot.send_message(chat_id, "Case chưa có worker.").await?;
        return Ok(());
    };
    let fee_percent = platform_fee_percent(&ctx);
    let platform_fee = case.amount * fee_percent / 100;
    let worker_payout = case.amount - platform_fee;
    let now = Utc::now().to_rfc3339();
    let mut tx = ctx.pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE facebook_unlock_cases
         SET status = 'completed', platform_fee = ?, worker_payout = ?, completed_at = ?, payout_at = ?, updated_at = ?
         WHERE id = ? AND status IN ('paid_in_progress', 'worker_done')",
    )
    .bind(platform_fee)
    .bind(worker_payout)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(&case.id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        ctx.bot.send_message(chat_id, "Case này không còn ở trạng thái có thể hoàn tất.").await?;
        return Ok(());
    }
    let worker_balance = wallet_repo::credit_wallet(
        &mut tx,
        worker_user_id,
        worker_payout,
        "facebook_unlock_payout",
        Some(&case.id),
        None,
        Some("facebook_unlock_worker_payout"),
    )
    .await?;
    tx.commit().await?;

    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Case <code>{}</code> đã hoàn tất. Cảm ơn bạn đã xác nhận.",
                html_escape(case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    if let Some(quote_id) = case.accepted_quote_id.as_deref() {
        if let Some(quote) = load_quote(&ctx.pool, quote_id).await? {
            let _ = ctx
                .bot
                .send_message(
                    ChatId(quote.worker_chat_id),
                    format!(
                        "✅ Case <code>{}</code> đã hoàn tất. Bạn nhận <b>{}</b>. Số dư ví: {}.",
                        html_escape(case_id),
                        format_vnd(worker_payout),
                        format_vnd(worker_balance)
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
    }
    Ok(())
}

async fn request_cancel_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền hủy case này.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    if case.status == "open" || case.status == "quoted_accepted" {
        sqlx::query("UPDATE facebook_unlock_cases SET status = 'cancelled', updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(case_id)
            .execute(&ctx.pool)
            .await?;
        ctx.bot.send_message(chat_id, "Đã hủy case. Bạn chưa bị trừ tiền nên không cần hoàn tiền.").await?;
        return Ok(());
    }
    if case.status == "paid_in_progress" || case.status == "worker_done" || case.status == "worker_failed" {
        sqlx::query("UPDATE facebook_unlock_cases SET status = 'cancel_requested', updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(case_id)
            .execute(&ctx.pool)
            .await?;
        ctx.bot.send_message(chat_id, "Đã gửi yêu cầu hủy/hoàn tiền cho admin.").await?;
        notify_admins_cancel_requested(&ctx, &case).await;
        return Ok(());
    }
    ctx.bot.send_message(chat_id, "Case hiện không thể hủy.").await?;
    Ok(())
}

async fn dispute_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    customer_user_id: i64,
    case_id: &str,
) -> Result<()> {
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !is_case_customer(&case, customer_user_id, chat_id) {
        ctx.bot.send_message(chat_id, "Bạn không thể khiếu nại case này.").await?;
        return Ok(());
    }
    if !matches!(case.status.as_str(), "paid_in_progress" | "worker_done") {
        ctx.bot
            .send_message(chat_id, "Case này không còn ở trạng thái có thể khiếu nại.")
            .await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE facebook_unlock_cases SET status = 'disputed', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot.send_message(chat_id, "Đã gửi khiếu nại cho admin.").await?;
    notify_admins_dispute(&ctx, &case).await;
    Ok(())
}

async fn admin_refund_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    case_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền hoàn tiền.").await?;
        return Ok(());
    }
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if !matches!(case.status.as_str(), "cancel_requested" | "worker_failed" | "disputed") {
        ctx.bot.send_message(chat_id, "Case này không ở trạng thái cần hoàn tiền.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    let mut tx = ctx.pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE facebook_unlock_cases
         SET status = 'refunded', refunded_at = ?, updated_at = ?
         WHERE id = ? AND status IN ('cancel_requested', 'worker_failed', 'disputed')",
    )
    .bind(&now)
    .bind(&now)
    .bind(&case.id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        ctx.bot.send_message(chat_id, "Case này không còn ở trạng thái cần hoàn tiền.").await?;
        return Ok(());
    }
    let balance_after = wallet_repo::credit_wallet(
        &mut tx,
        case.user_id,
        case.amount,
        "facebook_unlock_refund",
        Some(&case.id),
        None,
        Some("facebook_unlock_refund"),
    )
    .await?;
    if let Some(quote_id) = case.accepted_quote_id.as_deref() {
        sqlx::query("UPDATE facebook_unlock_quotes SET status = 'refunded', updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(quote_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    ctx.bot
        .send_message(
            chat_id,
            format!("Đã hoàn {} cho khách case <code>{}</code>.", format_vnd(case.amount), html_escape(case_id)),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    let _ = ctx
        .bot
        .send_message(
            ChatId(case.chat_id),
            format!(
                "✅ Case <code>{}</code> đã được hoàn tiền. Số dư ví hiện tại: {}.",
                html_escape(case_id),
                format_vnd(balance_after)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await;
    Ok(())
}

async fn admin_reject_refund(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    case_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền xử lý hoàn tiền.").await?;
        return Ok(());
    }
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.status != "cancel_requested" && case.status != "disputed" {
        ctx.bot.send_message(chat_id, "Case này không ở trạng thái yêu cầu hoàn.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE facebook_unlock_cases SET status = 'paid_in_progress', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot.send_message(chat_id, "Đã từ chối hoàn tiền, case quay lại đang xử lý.").await?;
    let _ = ctx
        .bot
        .send_message(ChatId(case.chat_id), format!("Admin đã từ chối hoàn tiền case <code>{}</code>.", html_escape(case_id)))
        .parse_mode(ParseMode::Html)
        .await;
    Ok(())
}

async fn admin_reopen_case(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    case_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền mở lại case.").await?;
        return Ok(());
    }
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    if case.status != "worker_failed" {
        ctx.bot.send_message(chat_id, "Chỉ mở lại case khi worker báo không xử lý được.").await?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE facebook_unlock_cases
         SET status = 'open', accepted_quote_id = NULL, worker_user_id = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(&now)
    .bind(case_id)
    .execute(&ctx.pool)
    .await?;
    sqlx::query("UPDATE facebook_unlock_quotes SET status = 'rejected', updated_at = ? WHERE case_id = ?")
        .bind(&now)
        .bind(case_id)
        .execute(&ctx.pool)
        .await?;
    ctx.bot.send_message(chat_id, "Đã mở lại case để worker khác báo giá.").await?;
    notify_workers_new_case(&ctx, &case).await;
    Ok(())
}

async fn admin_delete_customer_cases(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    admin_user_id: i64,
    case_id: &str,
) -> Result<()> {
    if !is_admin(&ctx, admin_user_id) {
        ctx.bot.send_message(chat_id, "Bạn không có quyền xóa case khách.").await?;
        return Ok(());
    }
    let Some(case) = load_case(&ctx.pool, case_id).await? else {
        ctx.bot.send_message(chat_id, "Không tìm thấy case.").await?;
        return Ok(());
    };
    let mut tx = ctx.pool.begin().await?;
    let case_ids = sqlx::query_scalar::<_, String>(
        "SELECT id FROM facebook_unlock_cases WHERE user_id = ? OR chat_id = ?",
    )
    .bind(case.user_id)
    .bind(case.chat_id)
    .fetch_all(&mut *tx)
    .await?;
    for id in &case_ids {
        sqlx::query("DELETE FROM facebook_unlock_messages WHERE case_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM facebook_unlock_quotes WHERE case_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let deleted = sqlx::query("DELETE FROM facebook_unlock_cases WHERE user_id = ? OR chat_id = ?")
        .bind(case.user_id)
        .bind(case.chat_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "Đã xóa {} case của khách {}.",
                deleted,
                html_escape(&case_customer_username(&case))
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn load_case(pool: &SqlitePool, case_id: &str) -> Result<Option<FacebookUnlockCase>> {
    let case = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT id, user_id, chat_id, username, issue, COALESCE(case_details, account_info, '') AS case_details,
                accepted_quote_id, worker_user_id, amount, status, created_at
         FROM facebook_unlock_cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await?;
    Ok(case)
}

async fn load_quote(pool: &SqlitePool, quote_id: &str) -> Result<Option<FacebookUnlockQuote>> {
    let quote = sqlx::query_as::<_, FacebookUnlockQuote>(
        "SELECT id, case_id, worker_user_id, worker_chat_id, worker_username, amount, note, status, created_at
         FROM facebook_unlock_quotes WHERE id = ?",
    )
    .bind(quote_id)
    .fetch_optional(pool)
    .await?;
    Ok(quote)
}

async fn list_open_cases(pool: &SqlitePool, limit: i64) -> Result<Vec<FacebookUnlockCase>> {
    let rows = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT id, user_id, chat_id, username, issue, COALESCE(case_details, account_info, '') AS case_details,
                accepted_quote_id, worker_user_id, amount, status, created_at
         FROM facebook_unlock_cases
         WHERE status = 'open'
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn list_worker_in_progress_cases(
    pool: &SqlitePool,
    worker_user_id: i64,
    worker_chat_id: i64,
    limit: i64,
) -> Result<Vec<FacebookUnlockCase>> {
    let rows = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT c.id, c.user_id, c.chat_id, c.username, c.issue,
                COALESCE(c.case_details, c.account_info, '') AS case_details,
                c.accepted_quote_id, c.worker_user_id, c.amount, c.status, c.created_at
         FROM facebook_unlock_cases c
         LEFT JOIN facebook_unlock_quotes q ON q.id = c.accepted_quote_id
         WHERE c.status = 'paid_in_progress'
           AND (
                c.worker_user_id = ?
                OR q.worker_user_id = ?
                OR q.worker_chat_id = ?
           )
         ORDER BY c.updated_at DESC, c.created_at DESC
         LIMIT ?",
    )
    .bind(worker_user_id)
    .bind(worker_user_id)
    .bind(worker_chat_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn list_customer_cases(
    pool: &SqlitePool,
    customer_user_id: i64,
    customer_chat_id: i64,
    limit: i64,
) -> Result<Vec<FacebookUnlockCase>> {
    let rows = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT id, user_id, chat_id, username, issue,
                COALESCE(case_details, account_info, '') AS case_details,
                accepted_quote_id, worker_user_id, amount, status, created_at
         FROM facebook_unlock_cases
         WHERE (user_id = ? OR chat_id = ?)
           AND COALESCE(customer_hidden, 0) = 0
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(customer_user_id)
    .bind(customer_chat_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn worker_can_handle_case(
    pool: &SqlitePool,
    case: &FacebookUnlockCase,
    worker_user_id: i64,
    worker_chat_id: i64,
) -> Result<bool> {
    if case.worker_user_id == Some(worker_user_id) {
        return Ok(true);
    }
    let Some(quote_id) = case.accepted_quote_id.as_deref() else {
        return Ok(false);
    };
    let allowed: i64 = sqlx::query_scalar(
        "SELECT COUNT(1)
         FROM facebook_unlock_quotes
         WHERE id = ?
           AND (worker_user_id = ? OR worker_chat_id = ?)
           AND status IN ('accepted', 'paid')",
    )
    .bind(quote_id)
    .bind(worker_user_id)
    .bind(worker_chat_id)
    .fetch_one(pool)
    .await?;
    Ok(allowed > 0)
}

async fn notify_customer_quote(
    ctx: &AppContext,
    case: &FacebookUnlockCase,
    quote: &FacebookUnlockQuote,
) -> Result<()> {
    let note = quote
        .note
        .as_deref()
        .map(|note| format!("\nGhi chú: {}", html_escape(note)))
        .unwrap_or_default();
    ctx.bot
        .send_message(
            ChatId(case.chat_id),
            format!(
                "💬 Case <code>{}</code> có báo giá mới.\n\nGiá xử lý: <b>{}</b>{}\n\nBạn có thể đồng ý báo giá này để tiến hành thanh toán trung gian qua bot.",
                html_escape(&case.id),
                format_vnd(quote.amount),
                note
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(quote_customer_keyboard(&quote.id))
        .await?;
    Ok(())
}

async fn notify_workers_new_case(ctx: &AppContext, case: &FacebookUnlockCase) {
    let mut worker_ids = approved_worker_chat_ids(&ctx.pool).await.unwrap_or_default();
    for seed_id in worker_notification_ids(ctx) {
        if !worker_ids.contains(&seed_id) {
            worker_ids.push(seed_id);
        }
    }
    for worker_id in worker_ids {
        let note = html_escape(&case_note_info(&case.case_details));
        let text = format!(
            "🔓 <b>CASE CHỜ DỊCH VỤ</b>\n\n\
             ╭─ 🧾 <code>{}</code>\n\
             ├ 🔒 Vấn đề: {}\n\
             ├ 📝 Ghi chú: <i>{}</i>\n\
             ├ 🕓 Thời gian: {}\n\
             ╰────────────────",
            html_escape(&case.id),
            html_escape(&case.issue),
            note,
            html_escape(&format_short_time(&case.created_at)),
        );
        let _ = ctx
            .bot
            .send_message(ChatId(worker_id), text)
            .parse_mode(ParseMode::Html)
            .reply_markup(worker_quote_keyboard(&case.id))
            .await;
    }
}

async fn notify_admins_new_case(ctx: &AppContext, case: &FacebookUnlockCase) {
    for admin_id in notification_admin_ids(ctx) {
        let text = format!(
            "🔓 <b>CASE MỞ KHÓA FACEBOOK MỚI</b>\n\n\
             Mã case: <code>{}</code>\n\
             User ID: <code>{}</code>\n\
             Username: {}\n\
             Chat ID: <code>{}</code>\n\
             Trạng thái: <code>{}</code>\n\
             Thời gian: {}\n\n\
             <b>Vấn đề:</b>\n{}\n\n\
             <b>Thông tin public:</b>\n{}\n\n\
             <b>Thông tin case:</b>\n<pre>{}</pre>",
            html_escape(&case.id),
            case.user_id,
            html_escape(case.username.as_deref().unwrap_or("Không có")),
            case.chat_id,
            html_escape(&case.status),
            html_escape(&format_time(&case.created_at)),
            html_escape(&case.issue),
            html_escape(&public_case_info(&case.case_details)),
            html_escape(&case.case_details),
        );
        let _ = ctx
            .bot
            .send_message(ChatId(admin_id), text)
            .parse_mode(ParseMode::Html)
            .await;
    }
}

async fn notify_admins_quote(ctx: &AppContext, case: &FacebookUnlockCase, quote: &FacebookUnlockQuote) {
    for admin_id in notification_admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(
                ChatId(admin_id),
                format!(
                    "💬 <b>BÁO GIÁ MỞ KHÓA FACEBOOK</b>\n\nCase: <code>{}</code>\nQuote: <code>{}</code>\nWorker: <code>{}</code>\nGiá: <b>{}</b>\nTrạng thái: <code>{}</code>\nThời gian: {}",
                    html_escape(&case.id),
                    html_escape(&quote.id),
                    quote.worker_user_id,
                    format_vnd(quote.amount),
                    html_escape(&quote.status),
                    html_escape(&format_time(&quote.created_at))
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }
}

async fn notify_worker_quote_accepted(ctx: &AppContext, quote: &FacebookUnlockQuote) {
    let _ = ctx
        .bot
        .send_message(
            ChatId(quote.worker_chat_id),
            format!(
                "✅ Khách đã đồng ý báo giá <b>{}</b> cho case <code>{}</code>.\n\nVui lòng chờ khách thanh toán cho bot. Sau khi thanh toán, bot sẽ gửi case để bạn xử lý.",
                format_vnd(quote.amount),
                html_escape(&quote.case_id)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await;
}

async fn notify_worker_case_paid(ctx: &AppContext, case: &FacebookUnlockCase, quote: &FacebookUnlockQuote) {
    let fee_percent = platform_fee_percent(ctx);
    let platform_fee = quote.amount * fee_percent / 100;
    let worker_receive = quote.amount - platform_fee;
    let _ = ctx
        .bot
        .send_message(
            ChatId(quote.worker_chat_id),
            format!(
                "💰 <b>CASE ĐÃ THANH TOÁN</b>\n\n\
                 Case: <code>{}</code>\n\
                 Khách đã thanh toán: <b>{}</b>\n\
                 Phí sàn nội bộ: <b>{}%</b> = {}\n\
                 Bạn nhận dự kiến khi hoàn tất: <b>{}</b>\n\n\
                 <b>Vấn đề:</b>\n{}\n\n\
                 <b>Thông tin case:</b>\n<pre>{}</pre>",
                html_escape(&case.id),
                format_vnd(quote.amount),
                fee_percent,
                format_vnd(platform_fee),
                format_vnd(worker_receive),
                html_escape(&case.issue),
                html_escape(&case.case_details),
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(worker_paid_case_keyboard(ctx, "vi", &case.id))
        .await;
}

async fn notify_admins_case_paid(ctx: &AppContext, case: &FacebookUnlockCase, quote: &FacebookUnlockQuote) {
    let fee_percent = platform_fee_percent(ctx);
    let platform_fee = quote.amount * fee_percent / 100;
    let worker_receive = quote.amount - platform_fee;
    for admin_id in notification_admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(
                ChatId(admin_id),
                format!(
                    "💰 <b>CASE MỞ KHÓA ĐÃ THANH TOÁN</b>\n\nCase: <code>{}</code>\nQuote: <code>{}</code>\nKhách trả: <b>{}</b>\nWorker: <code>{}</code>\nPhí sàn {}%: {}\nWorker nhận dự kiến: {}",
                    html_escape(&case.id),
                    html_escape(&quote.id),
                    format_vnd(quote.amount),
                    quote.worker_user_id,
                    fee_percent,
                    format_vnd(platform_fee),
                    format_vnd(worker_receive)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }
}

async fn notify_admins_worker_application(
    ctx: &AppContext,
    application_id: &str,
    user_id: i64,
    username: Option<&str>,
    info: &str,
) {
    for admin_id in notification_admin_ids(ctx) {
        let text = format!(
            "🧑‍💻 <b>ĐĂNG KÝ DỊCH VỤ MỞ KHÓA FACEBOOK</b>\n\n\
             Mã đăng ký: <code>{}</code>\n\
             User ID: <code>{}</code>\n\
             Username: {}\n\n\
             <b>Thông tin:</b>\n<pre>{}</pre>",
            html_escape(application_id),
            user_id,
            html_escape(username.unwrap_or("Không có")),
            html_escape(info),
        );
        let _ = ctx
            .bot
            .send_message(ChatId(admin_id), text)
            .parse_mode(ParseMode::Html)
            .reply_markup(worker_application_keyboard(ctx, application_id))
            .await;
    }
}

fn parse_quote(raw: &str) -> Option<(i64, Option<String>)> {
    let mut parts = raw.splitn(2, '|');
    let amount_raw = parts
        .next()?
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let amount = amount_raw.parse::<i64>().ok().filter(|amount| *amount > 0)?;
    let note = parts
        .next()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(str::to_string);
    Some((amount, note))
}

fn topup_keyboard(ctx: &AppContext, lang: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        i18n::inline_button_callback(ctx, lang, "start_btn_topup", "💰 Nạp tiền", "wallet:topup"),
    ]])
}

fn case_created_keyboard(ctx: &AppContext, lang: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Ví tiền",
        "start:wallet",
    )]])
}

fn quote_customer_keyboard(quote_id: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![teloxide::types::InlineKeyboardButton::callback(
        "✅ Đồng ý báo giá",
        format!("fbunlock:accept_quote:{quote_id}"),
    )]])
}

fn pay_quote_keyboard(
    ctx: &AppContext,
    lang: &str,
    quote_id: &str,
    case_id: &str,
) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![
        vec![teloxide::types::InlineKeyboardButton::callback(
            "💳 Thanh toán trung gian",
            format!("fbunlock:pay_quote:{quote_id}"),
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_cancel_case",
            "❌ Hủy case",
            format!("fbunlock:cancel_case:{case_id}"),
        )],
    ])
}

fn worker_paid_case_keyboard(ctx: &AppContext, lang: &str, case_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_message_customer",
            "💬 Nhắn khách",
            format!("fbunlock:msg_customer:{case_id}"),
        )],
        vec![
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_worker_done",
                "✅ Báo đã hoàn tất",
                format!("fbunlock:worker_done:{case_id}"),
            ),
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_worker_failed",
                "⚠️ Không xử lý được",
                format!("fbunlock:worker_failed:{case_id}"),
            ),
        ],
    ])
}

fn customer_case_keyboard(ctx: &AppContext, lang: &str, case_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_message_worker",
            "💬 Nhắn dịch vụ",
            format!("fbunlock:msg_worker:{case_id}"),
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_cancel_case",
            "❌ Yêu cầu hủy/hoàn tiền",
            format!("fbunlock:cancel_case:{case_id}"),
        )],
    ])
}

fn customer_done_keyboard(ctx: &AppContext, lang: &str, case_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_confirm_done",
            "✅ Xác nhận thành công",
            format!("fbunlock:confirm_done:{case_id}"),
        )],
        vec![
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_dispute",
                "⚠️ Khiếu nại",
                format!("fbunlock:dispute:{case_id}"),
            ),
            i18n::inline_button_callback(
                ctx,
                lang,
                "fbunlock_btn_message_worker",
                "💬 Nhắn dịch vụ",
                format!("fbunlock:msg_worker:{case_id}"),
            ),
        ],
    ])
}

fn admin_refund_keyboard(
    ctx: &AppContext,
    lang: &str,
    case_id: &str,
    include_reopen: bool,
) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![
        i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_admin_refund",
            "💸 Hoàn tiền khách",
            format!("fbunlock:admin_refund:{case_id}"),
        ),
        i18n::inline_button_callback(
            ctx,
            lang,
            "fbunlock_btn_admin_reject_refund",
            "↩️ Từ chối hoàn",
            format!("fbunlock:admin_reject_refund:{case_id}"),
        ),
    ]];
    if include_reopen {
        rows.push(vec![InlineKeyboardButton::callback(
            "🔁 Mở lại case",
            format!("fbunlock:admin_reopen:{case_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "🗑️ Xóa toàn bộ case của khách",
        format!("fbunlock:admin_delete_customer_cases:{case_id}"),
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn worker_application_keyboard(ctx: &AppContext, application_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        i18n::inline_button_callback(
            ctx,
            "vi",
            "fbunlock_btn_approve_worker",
            "✅ Duyệt worker",
            format!("fbunlock:approve_worker:{application_id}"),
        ),
        i18n::inline_button_callback(
            ctx,
            "vi",
            "fbunlock_btn_reject_worker",
            "❌ Từ chối worker",
            format!("fbunlock:reject_worker:{application_id}"),
        ),
    ]])
}

fn worker_quote_keyboard(case_id: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![teloxide::types::InlineKeyboardButton::callback(
        "💬 Báo giá case",
        format!("fbunlock:quote:{case_id}"),
    )]])
}

fn format_time(value: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(value) else {
        return value.to_string();
    };
    let vietnam = FixedOffset::east_opt(7 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    dt.with_timezone(&vietnam).format("%d/%m/%Y %H:%M").to_string()
}

fn format_short_time(value: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(value) else {
        return value.to_string();
    };
    let vietnam = FixedOffset::east_opt(7 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    dt.with_timezone(&vietnam).format("%d/%m %H:%M").to_string()
}

fn public_case_info(case_details: &str) -> String {
    case_details
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && (line.to_ascii_lowercase().contains("uid")
                    || line.to_ascii_lowercase().contains("facebook")
                    || line.contains("fb.com"))
        })
        .or_else(|| case_details.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("Chưa có thông tin public")
        .to_string()
}

fn case_note_info(case_details: &str) -> String {
    case_details
        .lines()
        .find_map(|line| line.trim().strip_prefix("Thông tin khách note case:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .or_else(|| {
            case_details
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .next_back()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Khách chưa ghi note case.".to_string())
}

fn detail_value(case_details: &str, prefix: &str) -> Option<String> {
    case_details.lines().find_map(|line| {
        line.trim()
            .strip_prefix(prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn case_customer_username(case: &FacebookUnlockCase) -> String {
    detail_value(&case.case_details, "Telegram khách:")
        .or_else(|| case.username.as_deref().map(|username| format!("@{}", username.trim_start_matches('@'))))
        .unwrap_or_else(|| "Không có username".to_string())
}

async fn case_worker_username(ctx: &AppContext, case: &FacebookUnlockCase) -> String {
    let Some(quote_id) = case.accepted_quote_id.as_deref() else {
        return "Chưa có dịch vụ".to_string();
    };
    match load_quote(&ctx.pool, quote_id).await {
        Ok(Some(quote)) => quote
            .worker_username
            .as_deref()
            .map(|username| format!("@{}", username.trim_start_matches('@')))
            .unwrap_or_else(|| "Dịch vụ chưa có username".to_string()),
        _ => "Không tìm thấy dịch vụ".to_string(),
    }
}

fn customer_status_label(status: &str) -> &str {
    match status {
        "open" => "Chờ dịch vụ",
        "quoted_accepted" => "Chờ thanh toán",
        "paid_in_progress" => "Đang xử lý",
        "worker_done" => "Chờ xác nhận",
        "cancel_requested" => "Chờ admin",
        "disputed" => "Tranh chấp",
        "worker_failed" => "Dịch vụ báo lỗi",
        "completed" => "Hoàn tất",
        "refunded" => "Đã hoàn tiền",
        "cancelled" => "Đã hủy",
        other => other,
    }
}

fn short_button_text(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    let mut text = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        text.push_str("...");
    }
    if text.is_empty() {
        "case".to_string()
    } else {
        text
    }
}

fn short_id() -> String {
    Uuid::new_v4()
        .to_string()
        .chars()
        .filter(|ch| *ch != '-')
        .take(10)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn telegram_username_from_message(msg: &Message) -> Option<String> {
    msg.from()
        .and_then(|user| user.username.as_deref())
        .map(|username| format!("@{}", username.trim_start_matches('@')))
}

async fn validate_own_telegram_username(
    ctx: &AppContext,
    msg: &Message,
    raw: &str,
) -> Result<Option<String>> {
    let entered = raw.trim();
    if !entered.starts_with('@') || entered.len() <= 1 || entered.split_whitespace().count() != 1 {
        ctx.bot
            .send_message(msg.chat.id, "Username Telegram phải có dạng @tencuaban.")
            .await?;
        return Ok(None);
    }
    let Some(actual) = telegram_username_from_message(msg) else {
        ctx.bot
            .send_message(
                msg.chat.id,
                "Tài khoản Telegram của bạn chưa đặt username. Vui lòng đặt username Telegram trước rồi quay lại.",
            )
            .await?;
        return Ok(None);
    };
    if !entered.eq_ignore_ascii_case(&actual) {
        ctx.bot
            .send_message(
                msg.chat.id,
                format!(
                    "Username bạn nhập không trùng với Telegram của tài khoản đang dùng bot. Vui lòng nhập đúng {}.",
                    actual
                ),
            )
            .await?;
        return Ok(None);
    }
    Ok(Some(actual))
}

fn notification_admin_ids(ctx: &AppContext) -> Vec<i64> {
    let mut ids = ctx.order_notification_admin_ids();
    for admin_id in ctx.telegram_icon_admin_ids() {
        if !ids.contains(&admin_id) {
            ids.push(admin_id);
        }
    }
    ids
}

fn is_admin(ctx: &AppContext, user_id: i64) -> bool {
    notification_admin_ids(ctx).into_iter().any(|admin_id| admin_id == user_id)
}

fn is_case_customer(case: &FacebookUnlockCase, user_id: i64, chat_id: ChatId) -> bool {
    case.user_id == user_id || case.chat_id == chat_id.0
}

async fn is_approved_worker(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(1) FROM facebook_unlock_workers WHERE user_id = ? AND status = 'approved'",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(exists > 0)
}

async fn approved_worker_chat_ids(pool: &SqlitePool) -> Result<Vec<i64>> {
    let worker_ids = sqlx::query_scalar::<_, i64>(
        "SELECT chat_id FROM facebook_unlock_workers WHERE status = 'approved'",
    )
    .fetch_all(pool)
    .await?;
    Ok(worker_ids)
}

async fn load_worker_application(
    pool: &SqlitePool,
    application_id: &str,
) -> Result<Option<FacebookUnlockWorkerApplication>> {
    let application = sqlx::query_as::<_, FacebookUnlockWorkerApplication>(
        "SELECT id, user_id, chat_id, username, info
         FROM facebook_unlock_worker_applications WHERE id = ?",
    )
    .bind(application_id)
    .fetch_optional(pool)
    .await?;
    Ok(application)
}

async fn save_relay_message(
    pool: &SqlitePool,
    case_id: &str,
    sender_role: &str,
    sender_user_id: i64,
    message: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO facebook_unlock_messages (id, case_id, sender_role, sender_user_id, message, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("FBMSG-{}", short_id()))
    .bind(case_id)
    .bind(sender_role)
    .bind(sender_user_id)
    .bind(message)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

async fn notify_admins_cancel_requested(ctx: &AppContext, case: &FacebookUnlockCase) {
    let customer_username = case_customer_username(case);
    let worker_username = case_worker_username(ctx, case).await;
    for admin_id in notification_admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(
                ChatId(admin_id),
                format!(
                    "⚠️ <b>KHÁCH YÊU CẦU HỦY/HOÀN TIỀN</b>\n\nCase: <code>{}</code>\nKhách: {}\nDịch vụ: {}\nSố tiền: <b>{}</b>\nThời gian tạo: {}",
                    html_escape(&case.id),
                    html_escape(&customer_username),
                    html_escape(&worker_username),
                    format_vnd(case.amount),
                    html_escape(&format_time(&case.created_at))
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(admin_refund_keyboard(ctx, "vi", &case.id, false))
            .await;
    }
}

async fn notify_admins_worker_failed(ctx: &AppContext, case: &FacebookUnlockCase) {
    let customer_username = case_customer_username(case);
    let worker_username = case_worker_username(ctx, case).await;
    for admin_id in notification_admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(
                ChatId(admin_id),
                format!(
                    "⚠️ <b>WORKER BÁO KHÔNG XỬ LÝ ĐƯỢC</b>\n\nCase: <code>{}</code>\nKhách: {}\nDịch vụ: {}\nSố tiền: <b>{}</b>",
                    html_escape(&case.id),
                    html_escape(&customer_username),
                    html_escape(&worker_username),
                    format_vnd(case.amount)
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(admin_refund_keyboard(ctx, "vi", &case.id, true))
            .await;
    }
}

async fn notify_admins_dispute(ctx: &AppContext, case: &FacebookUnlockCase) {
    let customer_username = case_customer_username(case);
    let worker_username = case_worker_username(ctx, case).await;
    for admin_id in notification_admin_ids(ctx) {
        let _ = ctx
            .bot
            .send_message(
                ChatId(admin_id),
                format!(
                    "⚠️ <b>KHÁCH KHIẾU NẠI CASE MỞ KHÓA FACEBOOK</b>\n\nCase: <code>{}</code>\nKhách: {}\nDịch vụ: {}\nSố tiền: <b>{}</b>",
                    html_escape(&case.id),
                    html_escape(&customer_username),
                    html_escape(&worker_username),
                    format_vnd(case.amount)
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(admin_refund_keyboard(ctx, "vi", &case.id, false))
            .await;
    }
}

fn worker_notification_ids(ctx: &AppContext) -> Vec<i64> {
    let mut ids = notification_admin_ids(ctx);
    for worker_id in ctx
        .get_text("facebook_unlock_worker_ids", "")
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter_map(|raw| raw.trim().parse::<i64>().ok())
    {
        if !ids.contains(&worker_id) {
            ids.push(worker_id);
        }
    }
    ids
}
