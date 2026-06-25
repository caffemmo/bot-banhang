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

const DEFAULT_PLATFORM_FEE_PERCENT: i64 = 20;

#[derive(Debug, Clone, FromRow)]
struct FacebookUnlockCase {
    id: String,
    user_id: i64,
    chat_id: i64,
    username: Option<String>,
    issue: String,
    case_details: String,
    accepted_quote_id: Option<String>,
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
            State::FacebookUnlockQuote { case_id } => {
                if text.is_empty() {
                    ask_quote_amount(&ctx, msg.chat.id, &case_id).await?;
                    return Ok(true);
                }
                submit_quote(ctx.clone(), &msg, &case_id, text, &lang).await?;
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
            "fbunlock:worker_cases" => {
                send_worker_case_list(&ctx, chat_id, &lang).await?;
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
         Ví dụ báo giá 300.000đ, phí sàn {}%, bạn nhận dự kiến 240.000đ sau khi hoàn tất.",
        fee_percent
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
    case_details: String,
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
    .bind(case_details.trim())
    .bind(case_details.trim())
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
        .reply_markup(case_created_keyboard(ctx, lang))
        .await?;

    if let Some(case) = load_case(&ctx.pool, &case_id).await? {
        notify_admins_new_case(&ctx, &case).await;
        notify_workers_new_case(&ctx, &case).await;
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

async fn send_worker_case_list(ctx: &AppContext, chat_id: ChatId, lang: &str) -> Result<()> {
    let cases = list_open_cases(&ctx.pool, 8).await?;
    if cases.is_empty() {
        ctx.bot
            .send_message(chat_id, "Hiện chưa có case nào đang chờ báo giá.")
            .await?;
        return Ok(());
    }

    let mut lines = vec!["📋 <b>CASE ĐANG CHỜ BÁO GIÁ</b>".to_string()];
    let mut rows = Vec::new();
    for case in cases {
        lines.push(format!(
            "\n<code>{}</code>\nVấn đề: {}\nTạo lúc: {}",
            html_escape(&case.id),
            html_escape(&case.issue),
            html_escape(&case.created_at)
        ));
        rows.push(vec![i18n::inline_button_callback_json(
            ctx,
            lang,
            "fbunlock_btn_quote_case",
            "💬 Báo giá case",
            format!("fbunlock:quote:{}", case.id),
        )]);
    }
    rows.push(vec![i18n::inline_button_callback_json(
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
        .reply_markup(pay_quote_keyboard(lang, &quote.id))
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
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        user_id,
        quote.amount,
        &case.id,
        Some("facebook_unlock_escrow"),
    )
    .await?;
    sqlx::query(
        "UPDATE facebook_unlock_cases SET status = 'paid_in_progress', amount = ?, worker_user_id = ?, updated_at = ? WHERE id = ?",
    )
    .bind(quote.amount)
    .bind(quote.worker_user_id)
    .bind(&now)
    .bind(&case.id)
    .execute(&mut *tx)
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
        .await?;

    notify_worker_case_paid(&ctx, &case, &quote).await;
    notify_admins_case_paid(&ctx, &case, &quote).await;
    Ok(())
}

async fn load_case(pool: &SqlitePool, case_id: &str) -> Result<Option<FacebookUnlockCase>> {
    let case = sqlx::query_as::<_, FacebookUnlockCase>(
        "SELECT id, user_id, chat_id, username, issue, COALESCE(case_details, account_info, '') AS case_details,
                accepted_quote_id, amount, status, created_at
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
                accepted_quote_id, amount, status, created_at
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
    for worker_id in worker_notification_ids(ctx) {
        let text = format!(
            "🔓 <b>CASE MỞ KHÓA FACEBOOK CẦN BÁO GIÁ</b>\n\n\
             Mã case: <code>{}</code>\n\
             Vấn đề: {}\n\
             Thời gian: {}\n\n\
             <b>Thông tin case:</b>\n<pre>{}</pre>",
            html_escape(&case.id),
            html_escape(&case.issue),
            html_escape(&case.created_at),
            html_escape(&case.case_details),
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
             <b>Thông tin case:</b>\n<pre>{}</pre>",
            html_escape(&case.id),
            case.user_id,
            html_escape(case.username.as_deref().unwrap_or("Không có")),
            case.chat_id,
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
                    html_escape(&quote.created_at)
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

fn pay_quote_keyboard(lang: &str, quote_id: &str) -> teloxide::types::InlineKeyboardMarkup {
    let _ = lang;
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![teloxide::types::InlineKeyboardButton::callback(
        "💳 Thanh toán trung gian",
        format!("fbunlock:pay_quote:{quote_id}"),
    )]])
}

fn worker_quote_keyboard(case_id: &str) -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![teloxide::types::InlineKeyboardButton::callback(
        "💬 Báo giá case",
        format!("fbunlock:quote:{case_id}"),
    )]])
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
