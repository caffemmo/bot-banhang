use std::collections::{BTreeSet, HashSet};
use std::env;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool, migrate::MigrateDatabase};
use teloxide::dispatching::UpdateFilterExt;
use teloxide::dptree;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message,
    MessageId,
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone)]
struct SupportConfig {
    token: String,
    database_url: String,
    manager_ids: Arc<HashSet<i64>>,
    agent_ids: Arc<HashSet<i64>>,
    case_prefix: String,
    overdue_minutes: i64,
}

#[derive(Clone)]
struct SupportContext {
    pool: SqlitePool,
    config: SupportConfig,
    team: Arc<RwLock<SupportTeam>>,
}

#[derive(Clone)]
struct SupportTeam {
    manager_ids: HashSet<i64>,
    agent_ids: HashSet<i64>,
    overdue_minutes: i64,
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
    assigned_agent_id: Option<i64>,
    assigned_agent_name: Option<String>,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct SupportCaseMessage {
    direction: String,
    source_chat_id: i64,
    source_message_id: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = SupportConfig::from_env()?;
    let pool = init_pool(&config.database_url).await?;
    let team = load_support_team(&pool, &config).await?;
    let bot = Bot::new(config.token.clone());
    let me = bot.get_me().await?;
    register_commands(&bot).await?;
    tracing::info!(
        "Support bot started as @{} with {} manager(s) and {} agent(s)",
        me.user.username.unwrap_or_default(),
        team.manager_ids.len(),
        team.agent_ids.len()
    );

    let ctx = Arc::new(SupportContext {
        pool,
        config,
        team: Arc::new(RwLock::new(team)),
    });
    spawn_team_config_refresh(ctx.clone());
    Dispatcher::builder(
        bot,
        dptree::entry()
            .branch(Update::filter_message().endpoint(handle_message))
            .branch(Update::filter_callback_query().endpoint(handle_callback)),
    )
        .dependencies(dptree::deps![ctx])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

impl SupportConfig {
    fn from_env() -> Result<Self> {
        let legacy_admin_ids = optional_env("SUPPORT_ADMIN_IDS")
            .map(|value| parse_ids(&value))
            .unwrap_or_default();
        let manager_ids = optional_env("SUPPORT_MANAGER_IDS")
            .map(|value| parse_ids(&value))
            .filter(|ids| !ids.is_empty())
            .unwrap_or_else(|| legacy_admin_ids.clone());
        let agent_ids = optional_env("SUPPORT_AGENT_IDS")
            .map(|value| parse_ids(&value))
            .filter(|ids| !ids.is_empty())
            .unwrap_or_else(|| manager_ids.clone());
        let case_prefix = env::var("SUPPORT_CASE_PREFIX")
            .unwrap_or_else(|_| "SUP".to_string())
            .trim()
            .to_ascii_uppercase();
        if case_prefix.is_empty()
            || case_prefix.len() > 12
            || !case_prefix.bytes().all(|ch| ch.is_ascii_alphanumeric())
        {
            return Err(anyhow!("SUPPORT_CASE_PREFIX must contain 1-12 letters or digits"));
        }

        Ok(Self {
            token: required_env("SUPPORT_BOT_TOKEN")?,
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://shop.db".to_string()),
            manager_ids: Arc::new(manager_ids),
            agent_ids: Arc::new(agent_ids),
            case_prefix,
            overdue_minutes: parse_overdue_minutes(),
        })
    }
}

impl SupportContext {
    fn team(&self) -> SupportTeam {
        self.team.read().expect("support team config lock poisoned").clone()
    }
}

fn spawn_team_config_refresh(ctx: Arc<SupportContext>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.tick().await;
        loop {
            interval.tick().await;
            match load_support_team(&ctx.pool, &ctx.config).await {
                Ok(team) => {
                    *ctx.team.write().expect("support team config lock poisoned") = team;
                }
                Err(err) => tracing::warn!("Could not refresh support team config: {err}"),
            }
        }
    });
}

async fn load_support_team(pool: &SqlitePool, config: &SupportConfig) -> Result<SupportTeam> {
    let values: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_configs WHERE key IN ('support_manager_ids', 'support_agent_ids', 'support_overdue_minutes')",
    )
    .fetch_all(pool)
    .await?;
    let values = values.into_iter().collect::<std::collections::HashMap<_, _>>();
    let manager_ids = values
        .get("support_manager_ids")
        .map(|value| parse_ids(value))
        .filter(|ids| !ids.is_empty())
        .unwrap_or_else(|| (*config.manager_ids).clone());
    let agent_ids = values
        .get("support_agent_ids")
        .map(|value| parse_ids(value))
        .filter(|ids| !ids.is_empty())
        .unwrap_or_else(|| (*config.agent_ids).clone());
    let overdue_minutes = values
        .get("support_overdue_minutes")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|minutes| (5..=1440).contains(minutes))
        .unwrap_or(config.overdue_minutes);
    if manager_ids.is_empty() && agent_ids.is_empty() {
        return Err(anyhow!(
            "No support team configured: save support_manager_ids or support_agent_ids in Admin"
        ));
    }
    Ok(SupportTeam {
        manager_ids,
        agent_ids,
        overdue_minutes,
    })
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
        bot_command("start", "Bắt đầu hỗ trợ"),
        bot_command("help", "Hướng dẫn sử dụng"),
    ])
    .await?;
    Ok(())
}

fn bot_command(command: &str, description: &str) -> BotCommand {
    BotCommand {
        command: command.to_string(),
        description: description.to_string(),
    }
}

async fn handle_message(bot: Bot, msg: Message, ctx: Arc<SupportContext>) -> Result<()> {
    if is_staff(&msg, &ctx) {
        handle_staff_message(&bot, &msg, &ctx).await
    } else {
        handle_customer_message(&bot, &msg, &ctx).await
    }
}

async fn handle_callback(bot: Bot, query: CallbackQuery, ctx: Arc<SupportContext>) -> Result<()> {
    let Some(data) = query.data.as_deref() else {
        return Ok(());
    };
    bot.answer_callback_query(query.id.clone()).await?;
    let user_id = query.from.id.0 as i64;
    let chat_id = query
        .message
        .as_ref()
        .map(|message| message.chat().id)
        .unwrap_or(ChatId(user_id));

    match data {
        "customer:close" => {
            close_customer_case_for_user(&bot, chat_id, user_id, &ctx).await?;
        }
        "staff:help" => {
            bot.send_message(chat_id, staff_help_text(is_manager(user_id, &ctx)))
                .reply_markup(staff_home_keyboard(is_manager(user_id, &ctx)))
                .await?;
        }
        "manager:summary" | "manager:new" | "manager:active" | "manager:overdue" | "manager:closed" => {
            if !is_manager(user_id, &ctx) {
                bot.send_message(chat_id, "Nút này chỉ dành cho trưởng nhóm.").await?;
            } else {
                let command = match data {
                    "manager:summary" => "/cases",
                    "manager:new" => "/new",
                    "manager:active" => "/active",
                    "manager:overdue" => "/overdue",
                    _ => "/closed",
                };
                send_manager_case_view(&bot, chat_id, command, &ctx).await?;
            }
        }
        _ if data.starts_with("case:") => {
            handle_case_callback(&bot, &query, chat_id, user_id, data, &ctx).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_case_callback(
    bot: &Bot,
    query: &CallbackQuery,
    chat_id: ChatId,
    user_id: i64,
    data: &str,
    ctx: &SupportContext,
) -> Result<()> {
    let mut parts = data.split(':');
    let _ = parts.next();
    let action = parts.next().unwrap_or_default();
    let case_id = parts.next().and_then(|value| value.parse::<i64>().ok());
    let Some(case_id) = case_id else {
        return Ok(());
    };
    let Some(case) = load_case_by_id(&ctx.pool, case_id).await? else {
        bot.send_message(chat_id, "Case không còn tồn tại.").await?;
        return Ok(());
    };

    match action {
        "claim" => {
            claim_case_for_user(bot, chat_id, user_id, &query.from.first_name, &case, ctx).await?;
        }
        "close" => {
            if !can_handle_case(user_id, &case, ctx) {
                bot.send_message(chat_id, "Case này đang do nhân viên khác phụ trách.").await?;
            } else {
                close_case_for_user(bot, chat_id, user_id, &case, ctx).await?;
            }
        }
        "transfer" => {
            if !can_handle_case(user_id, &case, ctx) {
                bot.send_message(chat_id, "Bạn không có quyền chuyển case này.").await?;
            } else {
                send_transfer_picker(bot, chat_id, &case, user_id, ctx).await?;
            }
        }
        "transfer_to" => {
            let target_id = parts.next().and_then(|value| value.parse::<i64>().ok());
            if let Some(target_id) = target_id {
                transfer_case_to(bot, chat_id, user_id, target_id, &case, ctx).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_customer_message(bot: &Bot, msg: &Message, ctx: &SupportContext) -> Result<()> {
    let command = command_name(msg);
    if matches!(command, Some("/start") | Some("/help")) {
        bot.send_message(
            msg.chat.id,
            "👋 Xin chào! Hãy cho chúng mình biết bạn cần hỗ trợ điều gì. Càng cung cấp nhiều thông tin (mã đơn hàng, ảnh lỗi, nội dung gặp phải...), đội ngũ hỗ trợ sẽ tiếp nhận và phản hồi trong thời gian sớm nhất ngay tại cuộc trò chuyện này.",
        )
        .reply_markup(customer_home_keyboard())
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
            format!("Đã tạo case #{}. Đội ngũ sẽ phản hồi cho bạn tại đây.", case.case_code),
        )
        .reply_markup(customer_home_keyboard())
        .await?;
        notify_staff_of_new_case(bot, &case, ctx).await?;
    }
    record_case_message(&ctx.pool, case.id, "customer", msg.chat.id.0, msg.id.0 as i64).await?;
    copy_customer_message_to_recipients(bot, msg, &case, ctx).await;
    Ok(())
}

async fn handle_staff_message(bot: &Bot, msg: &Message, ctx: &SupportContext) -> Result<()> {
    let command = command_name(msg);
    let user_id = sender_id(msg).unwrap_or_default();

    if matches!(command, Some("/start") | Some("/help")) {
        bot.send_message(msg.chat.id, staff_help_text(is_manager(user_id, ctx)))
            .reply_markup(staff_home_keyboard(is_manager(user_id, ctx)))
            .await?;
        return Ok(());
    }
    if is_manager(user_id, ctx) && matches!(command, Some("/cases") | Some("/new") | Some("/active") | Some("/overdue") | Some("/closed")) {
        send_manager_case_view(bot, msg.chat.id, command.unwrap_or("/cases"), ctx).await?;
        return Ok(());
    }

    let Some(reply_to) = msg.reply_to_message() else {
        if matches!(command, Some("/claim") | Some("/close") | Some("/transfer")) {
            bot.send_message(msg.chat.id, "Hãy trả lời vào nhãn hoặc nội dung của case rồi gửi lệnh.")
                .await?;
        }
        return Ok(());
    };
    let Some(case) = find_case_for_staff_reply(&ctx.pool, msg.chat.id.0, reply_to.id.0).await? else {
        bot.send_message(msg.chat.id, "Không tìm thấy case cho tin nhắn được trả lời.").await?;
        return Ok(());
    };

    if command == Some("/claim") {
        claim_case(bot, msg, &case, ctx).await?;
        return Ok(());
    }
    if command == Some("/transfer") {
        transfer_case(bot, msg, &case, ctx).await?;
        return Ok(());
    }
    if command == Some("/close") {
        close_case_for_user(bot, msg.chat.id, user_id, &case, ctx).await?;
        return Ok(());
    }
    if !can_handle_case(user_id, &case, ctx) {
        bot.send_message(msg.chat.id, "Case này đang do nhân viên khác phụ trách.").await?;
        return Ok(());
    }
    if command == Some("/close") {
        close_case(&ctx.pool, case.id).await?;
        bot.send_message(
            ChatId(case.user_chat_id),
            format!("Case #{} đã được đóng. Bạn có thể nhắn tin mới nếu cần thêm hỗ trợ.", case.case_code),
        )
        .await?;
        notify_case_closed(bot, &case, Some(user_id), ctx).await;
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
        tracing::warn!(case_code = %case.case_code, "Could not deliver staff reply: {err}");
        bot.send_message(msg.chat.id, format!("Không thể gửi phản hồi đến khách của case #{}.", case.case_code))
            .await?;
        return Ok(());
    }
    record_case_message(&ctx.pool, case.id, "agent", msg.chat.id.0, msg.id.0 as i64).await?;
    copy_staff_reply_to_managers(bot, msg, &case, ctx).await;
    Ok(())
}

async fn claim_case(bot: &Bot, msg: &Message, case: &SupportCase, ctx: &SupportContext) -> Result<()> {
    let agent_id = sender_id(msg).unwrap_or_default();
    if !is_agent(agent_id, ctx) {
        bot.send_message(msg.chat.id, "Chỉ nhân viên hỗ trợ mới có thể nhận case.").await?;
        return Ok(());
    }
    let agent_name = display_sender_name(msg);
    let result = sqlx::query(
        "UPDATE support_cases SET assigned_agent_id = ?, assigned_agent_name = ?, assigned_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND status = 'open' AND assigned_agent_id IS NULL",
    )
    .bind(agent_id)
    .bind(&agent_name)
    .bind(case.id)
    .execute(&ctx.pool)
    .await?;
    if result.rows_affected() == 0 {
        bot.send_message(msg.chat.id, "Case này đã được nhận hoặc đã đóng.").await?;
        return Ok(());
    }

    remove_case_from_other_agents(bot, case.id, agent_id, ctx).await;
    let updated = load_case_by_id(&ctx.pool, case.id).await?.context("Claimed case was not found")?;
    bot.send_message(msg.chat.id, format!("Bạn đã nhận case #{}.", updated.case_code)).await?;
    notify_managers(
        bot,
        format!("Case #{} đang do {} xử lý.", updated.case_code, agent_name),
        ctx,
    )
    .await;
    Ok(())
}

async fn transfer_case(bot: &Bot, msg: &Message, case: &SupportCase, ctx: &SupportContext) -> Result<()> {
    let sender = sender_id(msg).unwrap_or_default();
    if !can_handle_case(sender, case, ctx) {
        bot.send_message(msg.chat.id, "Bạn không có quyền chuyển case này.").await?;
        return Ok(());
    }
    let Some(target_id) = command_argument(msg).and_then(|value| value.parse::<i64>().ok()) else {
        bot.send_message(msg.chat.id, "Dùng: /transfer TELEGRAM_ID, khi trả lời vào case.").await?;
        return Ok(());
    };
    if !is_agent(target_id, ctx) {
        bot.send_message(msg.chat.id, "Telegram ID này không thuộc đội ngũ hỗ trợ.").await?;
        return Ok(());
    }
    if case.assigned_agent_id == Some(target_id) {
        bot.send_message(msg.chat.id, "Nhân viên này đang phụ trách case.").await?;
        return Ok(());
    }

    let target_name = format!("Nhân viên {target_id}");
    sqlx::query(
        "UPDATE support_cases SET assigned_agent_id = ?, assigned_agent_name = ?, assigned_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND status = 'open'",
    )
    .bind(target_id)
    .bind(&target_name)
    .bind(case.id)
    .execute(&ctx.pool)
    .await?;
    if let Some(previous_agent_id) = case.assigned_agent_id {
        if previous_agent_id != target_id && !is_manager(previous_agent_id, ctx) {
            remove_case_from_agent(bot, case.id, previous_agent_id, &ctx.pool).await;
        }
    }

    let updated = load_case_by_id(&ctx.pool, case.id).await?.context("Transferred case was not found")?;
    deliver_transferred_case(bot, &updated, target_id, ctx).await;
    bot.send_message(msg.chat.id, format!("Đã chuyển case #{} cho {}.", updated.case_code, target_name))
        .await?;
    notify_managers(
        bot,
        format!("Case #{} được chuyển cho {}.", updated.case_code, target_name),
        ctx,
    )
    .await;
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
    Ok((
        load_case_by_id(&ctx.pool, id)
            .await?
            .context("New support case was not found")?,
        true,
    ))
}

async fn notify_staff_of_new_case(bot: &Bot, case: &SupportCase, ctx: &SupportContext) -> Result<()> {
    let team = ctx.team();
    for manager_id in &team.manager_ids {
        send_case_header(bot, case, *manager_id, true, &ctx.pool).await?;
    }
    for agent_id in &team.agent_ids {
        if !team.manager_ids.contains(agent_id) {
            send_case_header(bot, case, *agent_id, false, &ctx.pool).await?;
        }
    }
    Ok(())
}

async fn send_case_header(
    bot: &Bot,
    case: &SupportCase,
    recipient_id: i64,
    is_manager_recipient: bool,
    pool: &SqlitePool,
) -> Result<()> {
    let action = if is_manager_recipient {
        "Bạn có thể theo dõi và hỗ trợ case này."
    } else {
        "Bấm nút Nhận case bên dưới để bắt đầu hỗ trợ."
    };
    let text = format!(
        "Case #{} mới\nKhách: {}\nTelegram ID: {}\n\n{}",
        case.case_code,
        customer_identity(case),
        case.user_id,
        action
    );
    let request = bot.send_message(ChatId(recipient_id), text).reply_markup(if is_manager_recipient {
        manager_case_keyboard(case.id)
    } else {
        new_case_keyboard(case.id)
    });
    match request.await {
        Ok(header) => record_staff_message(pool, case.id, recipient_id, header.id.0).await,
        Err(err) => {
            tracing::warn!(recipient_id, case_code = %case.case_code, "Could not notify support staff: {err}");
            Ok(())
        }
    }
}

async fn copy_customer_message_to_recipients(
    bot: &Bot,
    msg: &Message,
    case: &SupportCase,
    ctx: &SupportContext,
) {
    let label = format!(
        "📩 Case #{} | Khách: {}\nNội dung hoặc ảnh của khách ở ngay bên dưới.",
        case.case_code,
        customer_identity(case)
    );
    for recipient_id in case_recipients(case, ctx) {
        match bot
            .send_message(ChatId(recipient_id), &label)
            .reply_markup(case_message_keyboard(case, recipient_id, ctx))
            .await
        {
            Ok(marker) => {
                if let Err(err) = record_staff_message(&ctx.pool, case.id, recipient_id, marker.id.0).await {
                    tracing::warn!(recipient_id, case_code = %case.case_code, "Could not map case marker: {err}");
                }
            }
            Err(err) => tracing::warn!(recipient_id, case_code = %case.case_code, "Could not send case marker: {err}"),
        }
        match bot.copy_message(ChatId(recipient_id), msg.chat.id, msg.id).await {
            Ok(copied) => {
                if let Err(err) = record_staff_message(&ctx.pool, case.id, recipient_id, copied.0).await {
                    tracing::warn!(recipient_id, case_code = %case.case_code, "Could not map copied customer message: {err}");
                }
            }
            Err(err) => tracing::warn!(recipient_id, case_code = %case.case_code, "Could not copy customer message: {err}"),
        }
    }
}

async fn copy_staff_reply_to_managers(
    bot: &Bot,
    msg: &Message,
    case: &SupportCase,
    ctx: &SupportContext,
) {
    let sender_id = sender_id(msg).unwrap_or_default();
    let team = ctx.team();
    let label = format!(
        "📤 Case #{} | Phản hồi của {}",
        case.case_code,
        display_sender_name(msg)
    );
    for manager_id in &team.manager_ids {
        if *manager_id == sender_id {
            continue;
        }
        if let Ok(marker) = bot
            .send_message(ChatId(*manager_id), &label)
            .reply_markup(manager_case_keyboard(case.id))
            .await
        {
            let _ = record_staff_message(&ctx.pool, case.id, *manager_id, marker.id.0).await;
        }
        match bot.copy_message(ChatId(*manager_id), msg.chat.id, msg.id).await {
            Ok(copied) => {
                let _ = record_staff_message(&ctx.pool, case.id, *manager_id, copied.0).await;
            }
            Err(err) => tracing::warn!(manager_id, case_code = %case.case_code, "Could not copy staff reply to manager: {err}"),
        }
    }
}

async fn deliver_transferred_case(bot: &Bot, case: &SupportCase, target_id: i64, ctx: &SupportContext) {
    let header = format!(
        "Case #{} được chuyển cho bạn\nKhách: {}\n\nLịch sử trao đổi gần đây ở bên dưới.",
        case.case_code,
        customer_identity(case)
    );
    match bot
        .send_message(ChatId(target_id), header)
        .reply_markup(active_case_keyboard(case.id))
        .await
    {
        Ok(message) => {
            let _ = record_staff_message(&ctx.pool, case.id, target_id, message.id.0).await;
        }
        Err(err) => {
            tracing::warn!(target_id, case_code = %case.case_code, "Could not notify transfer recipient: {err}");
            return;
        }
    }
    let Ok(history) = recent_case_messages(&ctx.pool, case.id, 20).await else {
        return;
    };
    for item in history {
        let source_label = if item.direction == "customer" { "Khách" } else { "Nhân viên" };
        if let Ok(marker) = bot
            .send_message(ChatId(target_id), format!("Case #{} | Lịch sử từ {source_label}", case.case_code))
            .await
        {
            let _ = record_staff_message(&ctx.pool, case.id, target_id, marker.id.0).await;
        }
        let Ok(message_id) = i32::try_from(item.source_message_id) else {
            continue;
        };
        match bot.copy_message(ChatId(target_id), ChatId(item.source_chat_id), MessageId(message_id)).await {
            Ok(copied) => {
                let _ = record_staff_message(&ctx.pool, case.id, target_id, copied.0).await;
            }
            Err(err) => tracing::warn!(case_code = %case.case_code, "Could not copy transfer history: {err}"),
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
    notify_case_closed(bot, &case, None, ctx).await;
    Ok(())
}

async fn close_customer_case_for_user(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    ctx: &SupportContext,
) -> Result<()> {
    let Some(case) = find_open_case(&ctx.pool, chat_id.0).await? else {
        bot.send_message(chat_id, "Ban khong co case ho tro nao dang mo.").await?;
        return Ok(());
    };
    if case.user_id != user_id {
        bot.send_message(chat_id, "Ban khong co quyen dong case nay.").await?;
        return Ok(());
    }
    close_case(&ctx.pool, case.id).await?;
    bot.send_message(
        chat_id,
        format!("Case #{} da dong. Ban co the nhan tin moi neu can ho tro them.", case.case_code),
    )
    .reply_markup(customer_home_keyboard())
    .await?;
    notify_case_closed(bot, &case, None, ctx).await;
    Ok(())
}

async fn close_case_for_user(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    case: &SupportCase,
    ctx: &SupportContext,
) -> Result<()> {
    if !can_handle_case(user_id, case, ctx) {
        bot.send_message(chat_id, "Case nay dang do nhan vien khac phu trach.").await?;
        return Ok(());
    }
    if case.status != "open" {
        bot.send_message(chat_id, format!("Case #{} da dong.", case.case_code)).await?;
        return Ok(());
    }
    close_case(&ctx.pool, case.id).await?;
    bot.send_message(
        ChatId(case.user_chat_id),
        format!("Case #{} da duoc dong. Ban co the nhan tin moi neu can ho tro them.", case.case_code),
    )
    .await?;
    notify_case_closed(bot, case, Some(user_id), ctx).await;
    bot.send_message(chat_id, format!("Da dong case #{}.", case.case_code))
        .reply_markup(staff_home_keyboard(is_manager(user_id, ctx)))
        .await?;
    Ok(())
}

async fn claim_case_for_user(
    bot: &Bot,
    chat_id: ChatId,
    agent_id: i64,
    agent_name: &str,
    case: &SupportCase,
    ctx: &SupportContext,
) -> Result<()> {
    if !is_agent(agent_id, ctx) {
        bot.send_message(chat_id, "Chi nhan vien ho tro moi co the nhan case.").await?;
        return Ok(());
    }
    let result = sqlx::query(
        "UPDATE support_cases SET assigned_agent_id = ?, assigned_agent_name = ?, assigned_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND status = 'open' AND assigned_agent_id IS NULL",
    )
    .bind(agent_id)
    .bind(agent_name)
    .bind(case.id)
    .execute(&ctx.pool)
    .await?;
    if result.rows_affected() == 0 {
        bot.send_message(chat_id, "Case nay da duoc nhan hoac da dong.").await?;
        return Ok(());
    }
    remove_case_from_other_agents(bot, case.id, agent_id, ctx).await;
    let updated = load_case_by_id(&ctx.pool, case.id).await?.context("Claimed case was not found")?;
    bot.send_message(chat_id, format!("Ban da nhan case #{}.", updated.case_code))
        .reply_markup(active_case_keyboard(updated.id))
        .await?;
    notify_managers(bot, format!("Case #{} dang do {} xu ly.", updated.case_code, agent_name), ctx).await;
    Ok(())
}

async fn send_transfer_picker(
    bot: &Bot,
    chat_id: ChatId,
    case: &SupportCase,
    sender_id: i64,
    ctx: &SupportContext,
) -> Result<()> {
    if !can_handle_case(sender_id, case, ctx) {
        bot.send_message(chat_id, "Ban khong co quyen chuyen case nay.").await?;
        return Ok(());
    }
    let mut rows = Vec::new();
    for target_id in ctx.team().agent_ids {
        if target_id == sender_id || case.assigned_agent_id == Some(target_id) {
            continue;
        }
        rows.push(vec![InlineKeyboardButton::callback(
            format!("Nhan vien {target_id}"),
            format!("case:transfer_to:{}:{target_id}", case.id),
        )]);
    }
    if rows.is_empty() {
        bot.send_message(chat_id, "Chua co nhan vien khac de chuyen case.").await?;
    } else {
        bot.send_message(chat_id, format!("Chon nhan vien nhan case #{}:", case.case_code))
            .reply_markup(InlineKeyboardMarkup::new(rows))
            .await?;
    }
    Ok(())
}

async fn transfer_case_to(
    bot: &Bot,
    chat_id: ChatId,
    sender_id: i64,
    target_id: i64,
    case: &SupportCase,
    ctx: &SupportContext,
) -> Result<()> {
    if !can_handle_case(sender_id, case, ctx) {
        bot.send_message(chat_id, "Ban khong co quyen chuyen case nay.").await?;
        return Ok(());
    }
    if !is_agent(target_id, ctx) {
        bot.send_message(chat_id, "Nhan vien duoc chon khong con trong doi ho tro.").await?;
        return Ok(());
    }
    if case.assigned_agent_id == Some(target_id) {
        bot.send_message(chat_id, "Nhan vien nay dang phu trach case.").await?;
        return Ok(());
    }
    let target_name = format!("Nhan vien {target_id}");
    sqlx::query("UPDATE support_cases SET assigned_agent_id = ?, assigned_agent_name = ?, assigned_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND status = 'open'")
        .bind(target_id)
        .bind(&target_name)
        .bind(case.id)
        .execute(&ctx.pool)
        .await?;
    if let Some(previous_agent_id) = case.assigned_agent_id {
        if previous_agent_id != target_id && !is_manager(previous_agent_id, ctx) {
            remove_case_from_agent(bot, case.id, previous_agent_id, &ctx.pool).await;
        }
    }
    let updated = load_case_by_id(&ctx.pool, case.id).await?.context("Transferred case was not found")?;
    deliver_transferred_case(bot, &updated, target_id, ctx).await;
    bot.send_message(chat_id, format!("Da chuyen case #{} cho {}.", updated.case_code, target_name))
        .reply_markup(active_case_keyboard(updated.id))
        .await?;
    notify_managers(bot, format!("Case #{} duoc chuyen cho {}.", updated.case_code, target_name), ctx).await;
    Ok(())
}

fn customer_home_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Dong case hien tai",
        "customer:close",
    )]])
}

fn staff_home_keyboard(manager: bool) -> InlineKeyboardMarkup {
    if manager {
        InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback("Tong quan", "manager:summary")],
            vec![
                InlineKeyboardButton::callback("Case moi", "manager:new"),
                InlineKeyboardButton::callback("Dang xu ly", "manager:active"),
            ],
            vec![
                InlineKeyboardButton::callback("Qua han", "manager:overdue"),
                InlineKeyboardButton::callback("Da dong", "manager:closed"),
            ],
        ])
    } else {
        InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "Huong dan nhan case",
            "staff:help",
        )]])
    }
}

fn active_case_keyboard(case_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Dong case", format!("case:close:{case_id}")),
        InlineKeyboardButton::callback("Chuyen case", format!("case:transfer:{case_id}")),
    ]])
}

fn manager_case_keyboard(case_id: i64) -> InlineKeyboardMarkup {
    active_case_keyboard(case_id)
}

fn new_case_keyboard(case_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Nhan case",
        format!("case:claim:{case_id}"),
    )]])
}

fn case_message_keyboard(case: &SupportCase, recipient_id: i64, ctx: &SupportContext) -> InlineKeyboardMarkup {
    if is_manager(recipient_id, ctx) || case.assigned_agent_id == Some(recipient_id) {
        active_case_keyboard(case.id)
    } else {
        new_case_keyboard(case.id)
    }
}

async fn notify_case_closed(bot: &Bot, case: &SupportCase, actor_id: Option<i64>, ctx: &SupportContext) {
    let team = ctx.team();
    let mut recipients = BTreeSet::new();
    recipients.extend(team.manager_ids.iter().copied());
    if let Some(agent_id) = case.assigned_agent_id {
        recipients.insert(agent_id);
    }
    for recipient_id in recipients {
        if Some(recipient_id) != actor_id {
            let _ = bot.send_message(ChatId(recipient_id), format!("Case #{} đã được đóng.", case.case_code)).await;
        }
    }
}

async fn remove_case_from_other_agents(bot: &Bot, case_id: i64, assigned_agent_id: i64, ctx: &SupportContext) {
    let team = ctx.team();
    for agent_id in &team.agent_ids {
        if *agent_id != assigned_agent_id && !team.manager_ids.contains(agent_id) {
            remove_case_from_agent(bot, case_id, *agent_id, &ctx.pool).await;
        }
    }
}

async fn remove_case_from_agent(bot: &Bot, case_id: i64, agent_id: i64, pool: &SqlitePool) {
    let messages: Result<Vec<(i64,)>> = sqlx::query_as(
        "SELECT message_id FROM support_admin_messages WHERE case_id = ? AND admin_chat_id = ?",
    )
    .bind(case_id)
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into);
    if let Ok(messages) = messages {
        for (message_id,) in messages {
            if let Ok(message_id) = i32::try_from(message_id) {
                let _ = bot.delete_message(ChatId(agent_id), MessageId(message_id)).await;
            }
        }
    }
    let _ = sqlx::query("DELETE FROM support_admin_messages WHERE case_id = ? AND admin_chat_id = ?")
        .bind(case_id)
        .bind(agent_id)
        .execute(pool)
        .await;
}

async fn send_manager_case_view(bot: &Bot, chat_id: ChatId, command: &str, ctx: &SupportContext) -> Result<()> {
    let team = ctx.team();
    match command {
        "/cases" => {
            let summary = case_summary(&ctx.pool, team.overdue_minutes).await?;
            bot.send_message(
                chat_id,
                format!(
                    "Tổng quan hỗ trợ\n\nMới: {}\nĐang xử lý: {}\nQuá hạn: {}\nĐã đóng: {}\n\nChọn nhóm case bằng các nút bên dưới.",
                    summary.0, summary.1, summary.2, summary.3
                ),
            )
            .reply_markup(staff_home_keyboard(true))
            .await?;
        }
        "/new" => send_case_list(bot, chat_id, "Case mới", list_cases(&ctx.pool, "new", team.overdue_minutes).await?).await?,
        "/active" => send_case_list(bot, chat_id, "Case đang xử lý", list_cases(&ctx.pool, "active", team.overdue_minutes).await?).await?,
        "/overdue" => send_case_list(bot, chat_id, "Case quá hạn", list_cases(&ctx.pool, "overdue", team.overdue_minutes).await?).await?,
        "/closed" => send_case_list(bot, chat_id, "Case đã đóng", list_cases(&ctx.pool, "closed", team.overdue_minutes).await?).await?,
        _ => {}
    }
    Ok(())
}

async fn send_case_list(bot: &Bot, chat_id: ChatId, title: &str, cases: Vec<SupportCase>) -> Result<()> {
    if cases.is_empty() {
        bot.send_message(chat_id, format!("{title}: không có case nào."))
            .reply_markup(staff_home_keyboard(true))
            .await?;
        return Ok(());
    }
    let lines = cases
        .iter()
        .map(|case| {
            let owner = case.assigned_agent_name.as_deref().unwrap_or("Chưa có người nhận");
            format!("#{} | {} | {}", case.case_code, customer_identity(case), owner)
        })
        .collect::<Vec<_>>();
    bot.send_message(chat_id, format!("{title}\n\n{}", lines.join("\n")))
        .reply_markup(staff_home_keyboard(true))
        .await?;
    Ok(())
}

async fn find_open_case(pool: &SqlitePool, user_chat_id: i64) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT id, case_code, user_id, user_chat_id, user_name, username, status, assigned_agent_id, assigned_agent_name, updated_at FROM support_cases WHERE user_chat_id = ? AND status = 'open' LIMIT 1",
    )
    .bind(user_chat_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_case_by_id(pool: &SqlitePool, id: i64) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT id, case_code, user_id, user_chat_id, user_name, username, status, assigned_agent_id, assigned_agent_name, updated_at FROM support_cases WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn find_case_for_staff_reply(pool: &SqlitePool, staff_chat_id: i64, message_id: i32) -> Result<Option<SupportCase>> {
    sqlx::query_as::<_, SupportCase>(
        "SELECT c.id, c.case_code, c.user_id, c.user_chat_id, c.user_name, c.username, c.status, c.assigned_agent_id, c.assigned_agent_name, c.updated_at FROM support_admin_messages m JOIN support_cases c ON c.id = m.case_id WHERE m.admin_chat_id = ? AND m.message_id = ? LIMIT 1",
    )
    .bind(staff_chat_id)
    .bind(message_id as i64)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn record_staff_message(pool: &SqlitePool, case_id: i64, staff_chat_id: i64, message_id: i32) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO support_admin_messages (case_id, admin_chat_id, message_id) VALUES (?, ?, ?)")
        .bind(case_id)
        .bind(staff_chat_id)
        .bind(message_id as i64)
        .execute(pool)
        .await?;
    Ok(())
}

async fn record_case_message(
    pool: &SqlitePool,
    case_id: i64,
    direction: &str,
    source_chat_id: i64,
    source_message_id: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO support_case_messages (case_id, direction, source_chat_id, source_message_id) VALUES (?, ?, ?, ?)",
    )
    .bind(case_id)
    .bind(direction)
    .bind(source_chat_id)
    .bind(source_message_id)
    .execute(pool)
    .await?;
    let timestamp_column = if direction == "customer" { "last_customer_message_at" } else { "last_agent_reply_at" };
    sqlx::query(&format!(
        "UPDATE support_cases SET {timestamp_column} = datetime('now'), updated_at = datetime('now') WHERE id = ?"
    ))
    .bind(case_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn recent_case_messages(pool: &SqlitePool, case_id: i64, limit: i64) -> Result<Vec<SupportCaseMessage>> {
    sqlx::query_as::<_, SupportCaseMessage>(
        "SELECT direction, source_chat_id, source_message_id FROM (SELECT direction, source_chat_id, source_message_id, id FROM support_case_messages WHERE case_id = ? ORDER BY id DESC LIMIT ?) ORDER BY id ASC",
    )
    .bind(case_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn close_case(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("UPDATE support_cases SET status = 'closed', closed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn case_summary(pool: &SqlitePool, overdue_minutes: i64) -> Result<(i64, i64, i64, i64)> {
    let threshold = format!("-{overdue_minutes} minutes");
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(CASE WHEN status = 'open' AND assigned_agent_id IS NULL THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'open' AND assigned_agent_id IS NOT NULL THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'open' AND assigned_agent_id IS NOT NULL AND updated_at < datetime('now', ?) THEN 1 ELSE 0 END), 0), COALESCE(SUM(CASE WHEN status = 'closed' THEN 1 ELSE 0 END), 0) FROM support_cases",
    )
    .bind(threshold)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

async fn list_cases(pool: &SqlitePool, view: &str, overdue_minutes: i64) -> Result<Vec<SupportCase>> {
    let threshold = format!("-{overdue_minutes} minutes");
    let condition = match view {
        "new" => "status = 'open' AND assigned_agent_id IS NULL",
        "active" => "status = 'open' AND assigned_agent_id IS NOT NULL",
        "overdue" => "status = 'open' AND assigned_agent_id IS NOT NULL AND updated_at < datetime('now', ?)",
        "closed" => "status = 'closed'",
        _ => "0",
    };
    let sql = format!(
        "SELECT id, case_code, user_id, user_chat_id, user_name, username, status, assigned_agent_id, assigned_agent_name, updated_at FROM support_cases WHERE {condition} ORDER BY updated_at DESC LIMIT 30"
    );
    let mut query = sqlx::query_as::<_, SupportCase>(&sql);
    if view == "overdue" {
        query = query.bind(threshold);
    }
    query.fetch_all(pool).await.map_err(Into::into)
}

async fn notify_managers(bot: &Bot, text: String, ctx: &SupportContext) {
    let team = ctx.team();
    for manager_id in &team.manager_ids {
        let _ = bot.send_message(ChatId(*manager_id), &text).await;
    }
}

fn case_recipients(case: &SupportCase, ctx: &SupportContext) -> BTreeSet<i64> {
    let team = ctx.team();
    let mut recipients = BTreeSet::new();
    recipients.extend(team.manager_ids.iter().copied());
    if let Some(agent_id) = case.assigned_agent_id {
        recipients.insert(agent_id);
    } else {
        recipients.extend(team.agent_ids.iter().copied());
    }
    recipients
}

fn customer_identity(case: &SupportCase) -> String {
    let name = case.user_name.as_deref().filter(|name| !name.is_empty()).unwrap_or("Chưa có tên");
    match case.username.as_deref() {
        Some(username) if !username.is_empty() => format!("{name} (@{username})"),
        _ => name.to_string(),
    }
}

fn is_manager(user_id: i64, ctx: &SupportContext) -> bool {
    ctx.team().manager_ids.contains(&user_id)
}

fn is_agent(user_id: i64, ctx: &SupportContext) -> bool {
    ctx.team().agent_ids.contains(&user_id)
}

fn is_staff(msg: &Message, ctx: &SupportContext) -> bool {
    sender_id(msg).is_some_and(|user_id| is_manager(user_id, ctx) || is_agent(user_id, ctx))
}

fn can_handle_case(user_id: i64, case: &SupportCase, ctx: &SupportContext) -> bool {
    is_manager(user_id, ctx) || case.assigned_agent_id == Some(user_id)
}

fn sender_id(msg: &Message) -> Option<i64> {
    msg.from().map(|user| user.id.0 as i64)
}

fn display_sender_name(msg: &Message) -> String {
    msg.from()
        .map(|user| {
            [user.first_name.clone(), user.last_name.clone().unwrap_or_default()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Nhân viên hỗ trợ".to_string())
}

fn staff_help_text(manager: bool) -> &'static str {
    if manager {
        "Dùng các nút bên dưới để xem danh sách case. Trên từng case, bạn có thể đóng hoặc chuyển case cho nhân viên khác."
    } else {
        "Khi có case mới, bấm Nhận case. Sau khi nhận, chỉ bạn và trưởng nhóm nhận tin mới của khách; dùng các nút trên case để đóng hoặc chuyển."
    }
}

fn command_name(msg: &Message) -> Option<&str> {
    let command = msg.text()?.trim().split_whitespace().next()?;
    let command = command.split('@').next()?;
    command.starts_with('/').then_some(command)
}

fn command_argument(msg: &Message) -> Option<&str> {
    msg.text()?.trim().split_whitespace().nth(1)
}

fn required_env(key: &str) -> Result<String> {
    optional_env(key).context(format!("{key} is required"))
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn parse_ids(value: &str) -> HashSet<i64> {
    value
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter_map(|value| value.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect()
}

fn parse_overdue_minutes() -> i64 {
    optional_env("SUPPORT_OVERDUE_MINUTES")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|minutes| *minutes >= 5 && *minutes <= 1440)
        .unwrap_or(30)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_staff_ids_from_common_env_formats() {
        let ids = parse_ids("123, 456\n789 invalid 0 -1");
        assert_eq!(ids, HashSet::from([123, 456, 789]));
    }

    #[test]
    fn validates_overdue_window() {
        assert!((5..=1440).contains(&30));
        assert!(!(5..=1440).contains(&4));
    }
}
