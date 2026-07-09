use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message};
use tokio::time::{Duration, sleep};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};

const PREFIX: &str = "payblue_demo:";
const JOB_KIND_META_PAYMENT: &str = "meta_payment_demo";

#[derive(Debug, Clone, FromRow)]
struct DemoJob {
    id: i64,
    chat_id: i64,
    user_id: i64,
    kind: String,
    status: String,
    result: Option<String>,
    error: Option<String>,
    created_at: String,
    updated_at: String,
}

pub struct PayBlueDemoPlugin;

#[async_trait::async_trait]
impl AppPlugin for PayBlueDemoPlugin {
    fn name(&self) -> &'static str {
        "CmdPayBlueDemo"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_schema(pool).await
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_command(text, "/payblue_demo") {
            return Ok(false);
        }

        let Some(user) = msg.from() else {
            return Ok(true);
        };
        if !is_demo_admin(&ctx, user.id.0 as i64) {
            return Ok(true);
        }

        ensure_schema(&ctx.pool).await?;
        send_demo_menu(&ctx, msg.chat.id).await?;
        let _ = dialogue.update(State::Idle).await;
        Ok(true)
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
        if !data.starts_with(PREFIX) {
            return Ok(false);
        }

        if !is_demo_admin(&ctx, q.from.id.0 as i64) {
            let _ = ctx
                .bot
                .answer_callback_query(q.id.clone())
                .text("Bạn không có quyền dùng demo này.")
                .show_alert(true)
                .await;
            return Ok(true);
        }

        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };
        let chat_id = msg.chat().id;
        let user_id = q.from.id.0 as i64;
        ensure_schema(&ctx.pool).await?;

        match data.as_str() {
            "payblue_demo:menu" => send_demo_menu(&ctx, chat_id).await?,
            "payblue_demo:meta" => create_and_spawn_demo_job(
                ctx.clone(),
                chat_id,
                user_id,
                JOB_KIND_META_PAYMENT,
                "Thanh toán Meta Verified",
            )
            .await?,
            _ => send_demo_menu(&ctx, chat_id).await?,
        }

        let _ = dialogue.update(State::Idle).await;
        Ok(true)
    }
}

async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS payblue_demo_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            result TEXT,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_payblue_demo_jobs_user_created ON payblue_demo_jobs (user_id, created_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_payblue_demo_jobs_status ON payblue_demo_jobs (status, created_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn send_demo_menu(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    let text = "🧪 <b>PAY BLUE TICK DEMO</b>\nChỉ admin thấy bằng lệnh <code>/payblue_demo</code>.\n\nHiện chỉ bật thử logic nút <b>Thanh toán Meta</b>.";
    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("🚀 Thanh toán Meta", "payblue_demo:meta"),
        ]]))
        .await?;
    Ok(())
}

async fn create_and_spawn_demo_job(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    kind: &str,
    label: &str,
) -> Result<()> {
    let job = insert_demo_job(&ctx.pool, chat_id.0, user_id, kind).await?;
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Đã tạo job demo <code>#{}</code>\nLoại: <b>{}</b>\nTrạng thái: <b>Pending</b>",
                job.id, label
            ),
        )
        .parse_mode(teloxide::types::ParseMode::Html)
        .await?;

    tokio::spawn(async move {
        if let Err(err) = run_demo_job(ctx.clone(), job.id, ChatId(job.chat_id)).await {
            tracing::warn!("payblue demo job {} failed: {err}", job.id);
        }
    });
    Ok(())
}

async fn insert_demo_job(pool: &SqlitePool, chat_id: i64, user_id: i64, kind: &str) -> Result<DemoJob> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO payblue_demo_jobs (chat_id, user_id, kind, status, created_at, updated_at)
         VALUES (?, ?, ?, 'pending', ?, ?)",
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(kind)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(sqlx::query_as::<_, DemoJob>(
        r#"SELECT id, chat_id, user_id, kind, status, result, error, created_at, updated_at
           FROM payblue_demo_jobs
           WHERE chat_id = ? AND user_id = ? AND kind = ? AND created_at = ?
           ORDER BY id DESC
           LIMIT 1"#,
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(kind)
    .bind(now)
    .fetch_one(pool)
    .await?)
}

async fn run_demo_job(ctx: Arc<AppContext>, job_id: i64, chat_id: ChatId) -> Result<()> {
    sleep(Duration::from_secs(1)).await;
    update_demo_job_status(&ctx.pool, job_id, "running", None, None).await?;
    ctx.bot
        .send_message(chat_id, format!("⏳ Job demo <code>#{job_id}</code> đang xử lý..."))
        .parse_mode(teloxide::types::ParseMode::Html)
        .await?;

    sleep(Duration::from_secs(2)).await;
    update_demo_job_status(
        &ctx.pool,
        job_id,
        "success",
        Some("Demo worker xử lý thành công."),
        None,
    )
    .await?;
    ctx.bot
        .send_message(
            chat_id,
            format!("✅ Job demo <code>#{job_id}</code> thành công.\nKết quả: Demo worker xử lý thành công."),
        )
        .parse_mode(teloxide::types::ParseMode::Html)
        .await?;
    Ok(())
}

async fn update_demo_job_status(
    pool: &SqlitePool,
    job_id: i64,
    status: &str,
    result: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE payblue_demo_jobs SET status = ?, result = ?, error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(result)
    .bind(error)
    .bind(Utc::now().to_rfc3339())
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn is_demo_admin(ctx: &AppContext, user_id: i64) -> bool {
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
