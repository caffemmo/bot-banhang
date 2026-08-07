use std::collections::HashSet;
use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool, migrate::MigrateDatabase};
use teloxide::dispatching::UpdateFilterExt;
use teloxide::dptree;
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, ChatId, Message};
use uuid::Uuid;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct SupportConfig {
    token: String,
    database_url: String,
    admin_ids: Arc<HashSet<i64>>,
    case_prefix: String,
}

#[derive(Clone)]
struct SupportContext {
    pool: SqlitePool,
    config: SupportConfig,
}

#[derive(Debug, Clone, FromRow)]
struct SupportCase {
    id: i64,
    case_code: String,
    user_id: i64,
    user_chat_id: i64,
    user_name: Option<String>,
    username: Option<String>,
    status: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = SupportConfig::from_env()?;
    let pool = init_pool(&config.database_url).await?;
    let bot = Bot::new(config.token.clone());
    let me = bot.get_me().await?;
    register_commands(&bot).await?;

    tracing::info!(
        "Support bot started as @{} with {} admin(s)",
        me.user.username.unwrap_or_default(),
        config.admin_ids.len()
    );

    let ctx = Arc::new(SupportContext { pool, config });
    Dispatcher::builder(bot, Update::filter_message().endpoint(handle_message))
        .dependencies(dptree::deps![ctx])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

impl SupportConfig {
    fn from_env() -> Result<Self> {
        let token = required_env("SUPPORT_BOT_TOKEN")?;
        let admin_ids = parse_admin_ids(&required_env("SUPPORT_ADMIN_IDS")?);
        if admin_ids.is_empty() {
            return Err(anyhow!("SUPPORT_ADMIN_IDS must contain at least one Telegram user ID"));
        }

        let case_prefix = env::var("SUPPORT_CASE_PREFIX")
            .unwrap_or_else(|_| "SUP".to_string())
            .trim()
            .to_ascii_uppercase();
        if case_prefix.is_empty() || case_prefix.len() > 12 || !case_prefix.bytes().all(|ch| ch.is_ascii_alphanumeric()) {
            return Err(anyhow!("SUPPORT_CASE_PREFIX must contain 1-12 letters or digits"));
        }

        Ok(Self {
            token,
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://shop.db".to_string()),
            admin_ids: Arc::new(admin_ids),
            case_prefix,
        })
    }
}

async fn init_pool(database_url: &str) -> Result<SqlitePool> {
    if !sqlx::Sqlite::database_exists(database_url).await.unwrap_or(false) {
        sqlx::Sqlite::create_database(database_url).await?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;
    sqlx::query("PRAGMA busy_timeout = 5000").execute(&pool).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

async fn register_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(vec![
        BotCommand {
            command: "start".to_string(),
            description: "Bắt đầu hỗ trợ".to_string(),
        },
        BotCommand {
            command: "help".to_string(),
            description: "Hướng dẫn liên hệ hỗ trợ".to_string(),
        },
        BotCommand {
            command: "close".to_string(),
            description: "Đóng case hiện tại".to_string(),
        },
    ])
    .await?;
    Ok(())
}

async fn handle_message(bot: Bot, msg: Message, ctx: Arc<SupportContext>) -> Result<()> {
    if is_admin(&msg, &ctx) {
        handle_admin_message(&bot, &msg, &ctx).await
    } else {
        handle_customer_message(&bot, &msg, &ctx).await
    }
}

async fn handle_customer_message(bot: &Bot, msg: &Message, ctx: &SupportContext) -> Result<()> {
    let command = command_name(msg);
    if matches!(command, Some("/start") | Some("/help")) {
        bot.send_message(
            msg.chat.id,
            "👋 Xin chào! Hãy cho chúng mình biết bạn cần hỗ trợ điều gì. Càng cung cấp nhiều thông tin (mã đơn hàng, ảnh lỗi, nội dung gặp phải...), đội ngũ hỗ trợ sẽ tiếp nhận và phản hồi trong thời gian sớm nhất ngay tại cuộc trò chuyện này.",
        )
        .await?;
        return Ok(());
    }
    if command == Some("/close") {
        close_customer_case(bot, msg, ctx).await?;
        return Ok(());
    }
    if command.is_some() {
        bot.send_message(msg.chat.id, "Hãy gửi nội dung cần hỗ trợ hoặc dùng /help.")
            .await?;
        return Ok(());
    }

    let (case, is_new) = find_or_create_open_case(msg, ctx).await?;
    if is_new {
        bot.send_message(
            msg.chat.id,
            format!("Đã tạo case #{}. Admin sẽ phản hồi cho bạn tại đây.", case.case_code),
        )
        .await?;
        notify_admins_of_new_case(bot, &case, ctx).await?;
    }

    copy_customer_message_to_admins(bot, msg, &case, ctx).await;
    touch_case(&ctx.pool, case.id).await?;
    Ok(())
}

async fn handle_admin_message(bot: &Bot, msg: &Message, ctx: &SupportContext) -> Result<()> {
    let command = command_name(msg);
    if command == Some("/start") || command == Some("/help") {
        bot.send_message(
            msg.chat.id,
            "Trả lời trực tiếp vào thông báo đầu case để nhắn cho khách. Dùng /close khi trả lời vào case để đóng case; /cases để xem số case đang mở.",
        )
        .await?;
        return Ok(());
    }
    if command == Some("/cases") {
        let total = open_case_count(&ctx.pool).await?;
        bot.send_message(msg.chat.id, format!("Hiện có {total} case đang mở.")).await?;
        return Ok(());
    }

    let Some(reply_to) = msg.reply_to_message() else {
        if command == Some("/close") {
            bot.send_message(msg.chat.id, "Hãy trả lời vào thông báo đầu case rồi gửi /close.")
                .await?;
        }
        return Ok(());
    };
    let Some(case) = find_case_for_admin_reply(&ctx.pool, msg.chat.id.0, reply_to.id.0).await? else {
        if command == Some("/close") {
            bot.send_message(msg.chat.id, "Không tìm thấy case cho tin nhắn được trả lời.")
                .await?;
        }
        return Ok(());
    };

    if command == Some("/close") {
        close_case(&ctx.pool, case.id).await?;
        bot.send_message(ChatId(case.user_chat_id), format!("Case #{} đã được đóng. Bạn có thể nhắn tin mới nếu cần thêm hỗ trợ.", case.case_code))
            .await?;
        bot.send_message(msg.chat.id, format!("Đã đóng case #{}.", case.case_code)).await?;
        return Ok(());
    }
    if command.is_some() {
        return Ok(());
    }
    if case.status != "open" {
        bot.send_message(msg.chat.id, format!("Case #{} đã đóng. Không gửi phản hồi.", case.case_code))
            .await?;
        return Ok(());
    }

    if let Err(err) = bot.copy_message(ChatId(case.user_chat_id), msg.chat.id, msg.id).await {
        tracing::warn!(case_code = %case.case_code, "Could not deliver admin reply: {err}");
        bot.send_message(msg.chat.id, format!("Không thể gửi phản hồi đến khách của case #{}.", case.case_code))
            .await?;
        return Ok(());
    }
    touch_case(&ctx.pool, case.id).await?;
    Ok(())
}

async fn find_or_create_open_case(msg: &Message, ctx: &SupportContext) -> Result<(SupportCase, bool)> {
    if let Some(case) = find_open_case(&ctx.pool, msg.chat.id.0).await? {
        return Ok((case, false));
    }

    let from = msg.from().context("Support message without a Telegram sender")?;
    let user_name = [from.first_name.clone(), from.last_name.clone().unwrap_or_default()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let insert = sqlx::query(
        "INSERT INTO support_cases (case_code, user_id, user_chat_id, user_name, username) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(format!("PENDING-{}", Uuid::new_v4()))
    .bind(from.id.0 as i64)
    .bind(msg.chat.id.0)
    .bind(&user_name)
    .bind(&from.username)
    .execute(&ctx.pool)
    .await;

    let result = match insert {
        Ok(result) => result,
        Err(err) => {
            if let Some(case) = find_open_case(&ctx.pool, msg.chat.id.0).await? {
                return Ok((case, false));
            }
            return Err(err.into());
        }
    };
    let id = result.last_insert_rowid();
    let case_code = format!("{}-{}", ctx.config.case_prefix, id);
    sqlx::query("UPDATE support_cases SET case_code = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(&case_code)
        .bind(id)
        .execute(&ctx.pool)
        .await?;
    Ok((load_case_by_id(&ctx.pool, id).await?.context("New support case was not found")?, true))
}

async fn notify_admins_of_new_case(bot: &Bot, case: &SupportCase, ctx: &SupportContext) -> Result<()> {
    let display_name = case.user_name.as_deref().filter(|name| !name.is_empty()).unwrap_or("Chưa có tên");
    let username = case.username.as_deref().map(|name| format!("@{name}")).unwrap_or_else(|| "không có username".to_string());
    let text = format!(
        "Case #{} mới\nKhách: {} ({})\nTelegram ID: {}\n\nTrả lời trực tiếp vào tin nhắn này để gửi phản hồi cho khách.\nGửi /close khi trả lời vào đây để đóng case.",
        case.case_code, display_name, username, case.user_id
    );

    for admin_id in &*ctx.config.admin_ids {
        match bot.send_message(ChatId(*admin_id), &text).await {
            Ok(header) => record_admin_message(&ctx.pool, case.id, *admin_id, header.id.0).await?,
            Err(err) => tracing::warn!(admin_id, case_code = %case.case_code, "Could not notify support admin: {err}"),
        }
    }
    Ok(())
}

async fn copy_customer_message_to_admins(
    bot: &Bot,
    msg: &Message,
    case: &SupportCase,
    ctx: &SupportContext,
) {
    for admin_id in &*ctx.config.admin_ids {
        match bot.copy_message(ChatId(*admin_id), msg.chat.id, msg.id).await {
            Ok(copied) => {
                if let Err(err) = record_admin_message(&ctx.pool, case.id, *admin_id, copied.0).await {
                    tracing::warn!(admin_id, case_code = %case.case_code, "Could not map copied customer message: {err}");
                }
            }
            Err(err) => {
                tracing::warn!(admin_id, case_code = %case.case_code, "Could not copy customer message to admin: {err}");
            }
        }
    }
}

async fn close_customer_case(bot: &Bot, msg: &Message, ctx: &SupportContext) -> Result<()> {
    let Some(case) = find_open_case(&ctx.pool, msg.chat.id.0).await? else {
        bot.send_message(msg.chat.id, "Bạn không có case hỗ trợ nào đang mở.").await?;
        return Ok(());
    };
    close_case(&ctx.pool, case.id).await?;
    bot.send_message(msg.chat.id, format!("Đã đóng case #{}.", case.case_code)).await?;
    for admin_id in &*ctx.config.admin_ids {
        let _ = bot.send_message(ChatId(*admin_id), format!("Khách đã đóng case #{}.", case.case_code)).await;
    }
    Ok(())
}

async fn find_open_case(pool: &SqlitePool, user_chat_id: i64) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT id, case_code, user_id, user_chat_id, user_name, username, status FROM support_cases WHERE user_chat_id = ? AND status = 'open' LIMIT 1",
    )
    .bind(user_chat_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_case_by_id(pool: &SqlitePool, id: i64) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT id, case_code, user_id, user_chat_id, user_name, username, status FROM support_cases WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn find_case_for_admin_reply(pool: &SqlitePool, admin_chat_id: i64, message_id: i32) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT c.id, c.case_code, c.user_id, c.user_chat_id, c.user_name, c.username, c.status FROM support_admin_messages m JOIN support_cases c ON c.id = m.case_id WHERE m.admin_chat_id = ? AND m.message_id = ? LIMIT 1",
    )
    .bind(admin_chat_id)
    .bind(message_id as i64)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn record_admin_message(pool: &SqlitePool, case_id: i64, admin_chat_id: i64, message_id: i32) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO support_admin_messages (case_id, admin_chat_id, message_id) VALUES (?, ?, ?)")
        .bind(case_id)
        .bind(admin_chat_id)
        .bind(message_id as i64)
        .execute(pool)
        .await?;
    Ok(())
}

async fn close_case(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("UPDATE support_cases SET status = 'closed', closed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn touch_case(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("UPDATE support_cases SET updated_at = datetime('now') WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn open_case_count(pool: &SqlitePool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM support_cases WHERE status = 'open'")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

fn required_env(key: &str) -> Result<String> {
    env::var(key)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .context(format!("{key} is required"))
}

fn parse_admin_ids(value: &str) -> HashSet<i64> {
    value
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter_map(|value| value.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect()
}

fn is_admin(msg: &Message, ctx: &SupportContext) -> bool {
    msg.from().is_some_and(|user| ctx.config.admin_ids.contains(&(user.id.0 as i64)))
}

fn command_name(msg: &Message) -> Option<&str> {
    let command = msg.text()?.trim().split_whitespace().next()?;
    let command = command.split('@').next()?;
    command.starts_with('/').then_some(command)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_admin_ids_from_common_env_formats() {
        let ids = parse_admin_ids("123, 456\n789 invalid 0 -1");
        assert_eq!(ids, HashSet::from([123, 456, 789]));
    }

    #[test]
    fn accepts_compact_case_prefixes_only() {
        assert!("SUP12".bytes().all(|ch| ch.is_ascii_alphanumeric()));
        assert!(!"SUP-12".bytes().all(|ch| ch.is_ascii_alphanumeric()));
    }
}
