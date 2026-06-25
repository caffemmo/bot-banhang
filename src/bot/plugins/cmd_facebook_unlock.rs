use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, CallbackQuery, ChatId, Message, ParseMode};
use uuid::Uuid;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::{BotDialogue, State, chat_ui, i18n};
use crate::domains::wallet::repo as wallet_repo;

const DEFAULT_PRICE: i64 = 50_000;

#[derive(Debug, Clone, FromRow)]
struct FacebookUnlockCase {
    id: String,
    user_id: i64,
    chat_id: i64,
    username: Option<String>,
    issue: String,
    case_details: String,
    amount: i64,
    status: String,
    created_at: String,
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
            State::FacebookUnlockIssue => {
                if text.is_empty() {
                    ask_issue(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                dialogue
                    .update(State::FacebookUnlockDetails {
                        issue: text.to_string(),
                    })
                    .await?;
                ask_case_details(&ctx, msg.chat.id).await?;
                Ok(true)
            }
            State::FacebookUnlockDetails { issue } => {
                if text.is_empty() {
                    ask_case_details(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                submit_unlock_case(ctx.clone(), &msg, issue, text.to_string(), &lang).await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            State::FacebookUnlockWorkerApply => {
                if text.is_empty() {
                    ask_worker_application(&ctx, msg.chat.id).await?;
                    return Ok(true);
                }
                chat_ui::delete_message(&ctx, msg.chat.id, msg.id).await;
                submit_worker_application(ctx.clone(), &msg, text.to_string()).await?;
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
                ask_issue(&ctx, chat_id).await?;
                dialogue.update(State::FacebookUnlockIssue).await?;
            }
            "fbunlock:worker" => {
                send_worker_menu(&ctx, chat_id, &lang).await?;
                dialogue.update(State::Idle).await?;
            }
            "fbunlock:worker_apply" => {
                ask_worker_application(&ctx, chat_id).await?;
                dialogue.update(State::FacebookUnlockWorkerApply).await?;
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
            amount INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'paid_waiting_admin',
            worker_user_id INTEGER,
            worker_note TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query("ALTER TABLE facebook_unlock_cases ADD COLUMN case_details TEXT")
        .execute(pool)
        .await;

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

    Ok(())
}

fn service_price(ctx: &AppContext) -> i64 {
    ctx.get_text("facebook_unlock_price", &DEFAULT_PRICE.to_string())
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|amount| *amount >= 0)
        .unwrap_or(DEFAULT_PRICE)
}

fn button_matches(ctx: &AppContext, lang: &str, key: &str, text: &str) -> bool {
    i18n::button_text_match_variants(&i18n::t(ctx, lang, key, "🔓 Mở khóa Facebook"))
        .iter()
        .any(|variant| variant.eq_ignore_ascii_case(text))
}

async fn send_unlock_menu(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let price = service_price(ctx);
    let text = format!(
        "🔓 <b>MỞ KHÓA FACEBOOK</b>\n\n\
         Shop giữ tiền trung gian để khách và người làm dịch vụ giao dịch rõ ràng hơn.\n\n\
         • Khách gửi tình trạng tài khoản và thông tin cần xử lý.\n\
         • Bot thu phí trước: <b>{}</b>.\n\
         • Admin nhận case để kiểm tra và điều phối người làm dịch vụ.\n\n\
         Thông tin tài khoản bạn gửi sẽ được bot xóa khỏi chat sau khi nhận.",
        format_vnd(price)
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
                    [i18n::inline_button_callback_json(ctx, lang, "fbunlock_btn_customer", "🙋 Tôi cần mở khóa Facebook", "fbunlock:customer")],
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
    let text = "🧑‍💻 <b>KHU DỊCH VỤ MỞ KHÓA FACEBOOK</b>\n\n\
        Hiện tại shop đang nhận đăng ký người làm dịch vụ để admin duyệt thủ công.\n\n\
        Khi được duyệt, admin sẽ giao case phù hợp và giữ tiền trung gian đến khi có kết quả.";
    chat_ui::send_clean_menu_payload(
        ctx,
        chat_id,
        json!({
            "chat_id": chat_id.0,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": {
                "inline_keyboard": [
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

async fn ask_case_details(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "🔐 Vui lòng gửi thông tin theo mẫu:\n\nUID hoặc link Facebook:\nTên tài khoản nếu có:\nLiên hệ Telegram:\nGhi chú thêm:\n\nKhông gửi mật khẩu hoặc mã 2FA trong chat. Bot sẽ xóa tin nhắn này sau khi nhận để dọn chat.",
        )
        .await?;
    Ok(())
}

async fn ask_worker_application(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(
            chat_id,
            "📝 Vui lòng gửi thông tin đăng ký làm dịch vụ:\n\nKinh nghiệm:\nDịch vụ xử lý được:\nTỉ lệ nhận case:\nLiên hệ Telegram:\n\nBot sẽ xóa tin nhắn này sau khi nhận.",
        )
        .await?;
    Ok(())
}

async fn submit_unlock_case(
    ctx: Arc<AppContext>,
    msg: &Message,
    issue: String,
    case_details: String,
    lang: &str,
) -> Result<()> {
    let Some(user) = msg.from() else {
        ctx.bot
            .send_message(msg.chat.id, i18n::t(&ctx, lang, "user_unknown", "Cannot identify user."))
            .await?;
        return Ok(());
    };

    let user_id = user.id.0 as i64;
    let price = service_price(&ctx);
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;
    if wallet.balance < price {
        ctx.bot
            .send_message(
                msg.chat.id,
                format!(
                    "⚠️ Số dư ví không đủ.\nSố dư hiện tại: {}\nPhí mở khóa: {}\n\nVui lòng nạp thêm rồi tạo lại yêu cầu.",
                    format_vnd(wallet.balance),
                    format_vnd(price)
                ),
            )
            .reply_markup(topup_keyboard(&ctx, lang))
            .await?;
        return Ok(());
    }

    let case_id = format!("FBUNLOCK-{}", short_id());
    let now = Utc::now().to_rfc3339();
    let mut tx = ctx.pool.begin().await?;
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        user_id,
        price,
        &case_id,
        Some("Phí trung gian mở khóa Facebook"),
    )
    .await?;
    sqlx::query(
        r#"
        INSERT INTO facebook_unlock_cases
        (id, user_id, chat_id, username, issue, case_details, account_info, amount, status, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'paid_waiting_admin', ?, ?)
        "#,
    )
    .bind(&case_id)
    .bind(user_id)
    .bind(msg.chat.id.0)
    .bind(user.username.clone())
    .bind(issue.trim())
    .bind(case_details.trim())
    .bind(case_details.trim())
    .bind(price)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã nhận yêu cầu mở khóa Facebook.\n\nMã case: <code>{}</code>\nĐã thu: {}\nSố dư còn lại: {}\n\nAdmin sẽ kiểm tra và điều phối người làm dịch vụ. Bạn vui lòng chờ thông báo tiếp theo.",
                case_id,
                format_vnd(price),
                format_vnd(balance_after)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(case_done_keyboard(&ctx, lang))
        .await?;

    if let Some(case) = load_case(&ctx.pool, &case_id).await? {
        notify_admins_new_case(&ctx, &case).await;
    }

    Ok(())
}

async fn submit_worker_application(ctx: Arc<AppContext>, msg: &Message, info: String) -> Result<()> {
    let Some(user) = msg.from() else {
        ctx.bot.send_message(msg.chat.id, "Không xác định được user.").await?;
        return Ok(());
    };

    let application_id = format!("FBWORKER-{}", short_id());
    let now = Utc::now().to_rfc3339();
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
    .bind(info.trim())
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

async fn load_case(pool: &SqlitePool, case_id: &str) -> Result<Option<FacebookUnlockCase>> {
    let case = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT id, user_id, chat_id, username, issue, COALESCE(case_details, account_info, '') AS case_details, amount, status, created_at
         FROM facebook_unlock_cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await?;
    Ok(case)
}

async fn notify_admins_new_case(ctx: &AppContext, case: &FacebookUnlockCase) {
    for admin_id in notification_admin_ids(ctx) {
        let text = format!(
            "🔓 <b>CASE MỞ KHÓA FACEBOOK MỚI</b>\n\n\
             Mã case: <code>{}</code>\n\
             User ID: <code>{}</code>\n\
             Username: {}\n\
             Chat ID: <code>{}</code>\n\
             Phí đã thu: <b>{}</b>\n\
             Trạng thái: <code>{}</code>\n\
             Thời gian: {}\n\n\
             <b>Vấn đề:</b>\n{}\n\n\
             <b>Thông tin case:</b>\n<pre>{}</pre>",
            case.id,
            case.user_id,
            html_escape(case.username.as_deref().unwrap_or("Không có")),
            case.chat_id,
            format_vnd(case.amount),
            html_escape(&case.status),
            html_escape(&case.created_at),
            html_escape(&case.issue),
            html_escape(&case.case_details),
        );
        let _ = ctx
            .bot
            .send_message(ChatId(admin_id), text)
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
            .await;
    }
}

fn topup_keyboard(ctx: &AppContext, lang: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        i18n::inline_button_callback(ctx, lang, "start_btn_topup", "💰 Nạp tiền", "wallet:topup"),
    ]])
}

fn case_done_keyboard(ctx: &AppContext, lang: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_wallet",
            "💳 Ví tiền",
            "start:wallet",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_shop",
            "🛒 Xem sản phẩm",
            "start:shop",
        )],
    ])
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

fn notification_admin_ids(ctx: &AppContext) -> Vec<i64> {
    let mut ids = ctx.order_notification_admin_ids();
    for admin_id in ctx.telegram_icon_admin_ids() {
        if !ids.contains(&admin_id) {
            ids.push(admin_id);
        }
    }
    ids
}
