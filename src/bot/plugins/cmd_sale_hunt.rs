use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use rand::{Rng, distributions::Alphanumeric};
use serde::Serialize;
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::prelude::*;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message,
};
use tracing::{info, warn};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State, i18n};
use crate::domains::users::repo as users_repo;

const DEAL_TTL_MINUTES: i64 = 30;
const DAILY_CLAIM_LIMIT: i64 = 1;
const DISCOUNT_CHOICES: [i64; 8] = [5, 5, 7, 7, 10, 10, 12, 15];
const GOLDEN_HOUR_DURATION_MINUTES: i64 = 30;
const GOLDEN_HOUR_NOTIFY_BEFORE_MINUTES: i64 = 60;
const GOLDEN_HOUR_START_MINUTE: i64 = 9 * 60;
const GOLDEN_HOUR_END_START_MINUTE: i64 = 22 * 60 + 30;
const GOLDEN_HOUR_SLOT_MINUTES: i64 = 30;
const GOLDEN_HOUR_MAX_DISCOUNT: i64 = 7;
const GOLDEN_HOUR_DISCOUNTS: [i64; 5] = [3, 5, 5, 7, 7];

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SaleHuntDeal {
    pub id: i64,
    pub user_id: i64,
    pub chat_id: i64,
    pub code: String,
    pub discount_percent: i64,
    pub status: String,
    pub expires_at: String,
    pub order_id: Option<String>,
    pub created_at: String,
    pub used_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GoldenHourDeal {
    pub id: i64,
    pub deal_date: String,
    pub starts_at: String,
    pub ends_at: String,
    pub notify_at: String,
    pub discount_percent: i64,
    pub notified_at: Option<String>,
    pub created_at: String,
}

pub struct SaleHuntCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for SaleHuntCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdSaleHunt"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "sale".to_string(),
            description: "Săn sale".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_sale_hunt_text(&ctx, text) {
            return Ok(false);
        }
        let Some(user) = msg.from() else {
            return Ok(false);
        };
        let lang = i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await;
        show_sale_hunt(ctx, msg.chat.id, user.id.0 as i64, &lang).await?;
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
        if !data.starts_with("salehunt:") {
            return Ok(false);
        }
        let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };
        let chat_id = msg.chat().id;
        let user_id = q.from.id.0 as i64;

        match data.as_str() {
            "salehunt:hunt" => claim_sale_hunt(ctx, chat_id, user_id, &lang).await?,
            "salehunt:mine" => show_my_sale_hunt(ctx, chat_id, user_id, &lang).await?,
            _ => show_sale_hunt(ctx, chat_id, user_id, &lang).await?,
        }
        let _ = dialogue.update(State::Idle).await;
        Ok(true)
    }
}

pub async fn show_sale_hunt(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    ensure_schema(&ctx.pool).await?;
    expire_old_deals(&ctx.pool).await?;
    let golden_hour = ensure_next_golden_hour(&ctx.pool).await?;
    let active = active_deal_for_user(&ctx.pool, user_id).await?;
    let claims_today = count_claims_today(&ctx.pool, user_id).await?;
    let deal_line = active
        .as_ref()
        .map(|deal| active_deal_line(&ctx, lang, deal))
        .unwrap_or_else(|| {
            tl(
                &ctx,
                lang,
                "sale_hunt_no_active_deal",
                "Bạn chưa có deal đang hoạt động.",
            )
        });
    let golden_hour_line = render_golden_hour_line(&ctx, lang, &golden_hour);
    let mut text = trl(
        &ctx,
        lang,
        "sale_hunt_menu_text",
        "🔥 SĂN SALE HÔM NAY\n\nMỗi tài khoản săn được 1 deal/ngày. Deal áp dụng tự động cho đơn tiếp theo và hết hạn sau {ttl} phút.\n\n{deal_line}\n\nLượt hôm nay: {used}/{limit}",
        &[
            ("ttl", DEAL_TTL_MINUTES.to_string()),
            ("golden_hour_line", golden_hour_line),
            ("deal_line", deal_line),
            ("used", claims_today.min(DAILY_CLAIM_LIMIT).to_string()),
            ("limit", DAILY_CLAIM_LIMIT.to_string()),
        ],
    );
    if !text.contains(&golden_hour_line) {
        let marker = deal_line.as_str();
        text = if text.contains(marker) {
            text.replacen(marker, &format!("{golden_hour_line}\n\n{marker}"), 1)
        } else {
            format!("{golden_hour_line}\n\n{text}")
        };
    }
    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(sale_hunt_keyboard(&ctx, lang, claims_today < DAILY_CLAIM_LIMIT && active.is_none()))
        .await?;
    Ok(())
}

pub async fn active_deal_for_user(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<SaleHuntDeal>> {
    expire_old_deals(pool).await?;
    let now = Utc::now().to_rfc3339();
    let deal = sqlx::query_as::<_, SaleHuntDeal>(
        r#"SELECT id, user_id, chat_id, code, discount_percent, status, expires_at, order_id, created_at, used_at
           FROM sale_hunt_deals
           WHERE user_id = ? AND status = 'active' AND datetime(expires_at) > datetime(?)
           ORDER BY created_at DESC, id DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(deal)
}

pub async fn active_golden_hour_deal(pool: &SqlitePool) -> Result<Option<GoldenHourDeal>> {
    ensure_schema(pool).await?;
    let deal = ensure_next_golden_hour(pool).await?;
    let now = Utc::now().to_rfc3339();
    if datetime_before_or_equal(&deal.starts_at, &now) && datetime_before(&now, &deal.ends_at) {
        Ok(Some(deal))
    } else {
        Ok(None)
    }
}

pub fn golden_hour_discount_percent(deal: &GoldenHourDeal) -> i64 {
    deal.discount_percent.clamp(0, GOLDEN_HOUR_MAX_DISCOUNT)
}

pub fn discount_amount(amount: i64, discount_percent: i64) -> i64 {
    if amount <= 0 || discount_percent <= 0 {
        return 0;
    }
    (amount * discount_percent.clamp(0, 90) / 100).max(1)
}

pub async fn mark_deal_used_tx(
    tx: &mut Transaction<'_, Sqlite>,
    deal_id: i64,
    order_id: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE sale_hunt_deals SET status = 'used', order_id = ?, used_at = ? WHERE id = ? AND status = 'active'",
    )
    .bind(order_id)
    .bind(Utc::now().to_rfc3339())
    .bind(deal_id)
    .execute(tx.as_mut())
    .await?;
    Ok(())
}

async fn claim_sale_hunt(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    ensure_schema(&ctx.pool).await?;
    expire_old_deals(&ctx.pool).await?;

    if let Some(deal) = active_deal_for_user(&ctx.pool, user_id).await? {
        send_deal_result(&ctx, chat_id, lang, &deal, "sale_hunt_existing_deal").await?;
        return Ok(());
    }

    if count_claims_today(&ctx.pool, user_id).await? >= DAILY_CLAIM_LIMIT {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    lang,
                    "sale_hunt_daily_limit",
                    "Hôm nay bạn đã săn sale rồi. Hãy quay lại vào ngày mai nhé.",
                ),
            )
            .reply_markup(sale_hunt_result_keyboard(&ctx, lang))
            .await?;
        return Ok(());
    }

    let percent = random_discount_percent();
    let code = generate_code(percent);
    let expires_at = (Utc::now() + Duration::minutes(DEAL_TTL_MINUTES)).to_rfc3339();
    sqlx::query(
        "INSERT INTO sale_hunt_deals (user_id, chat_id, code, discount_percent, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(chat_id.0)
    .bind(&code)
    .bind(percent)
    .bind(&expires_at)
    .bind(Utc::now().to_rfc3339())
    .execute(&ctx.pool)
    .await?;

    let deal = active_deal_for_user(&ctx.pool, user_id)
        .await?
        .expect("fresh sale hunt deal should be active");
    send_deal_result(&ctx, chat_id, lang, &deal, "sale_hunt_claimed").await?;
    Ok(())
}

async fn show_my_sale_hunt(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    lang: &str,
) -> Result<()> {
    ensure_schema(&ctx.pool).await?;
    expire_old_deals(&ctx.pool).await?;
    if let Some(deal) = active_deal_for_user(&ctx.pool, user_id).await? {
        send_deal_result(&ctx, chat_id, lang, &deal, "sale_hunt_my_deal").await?;
    } else {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    lang,
                    "sale_hunt_no_deal",
                    "Bạn chưa có deal săn sale đang hoạt động.",
                ),
            )
            .reply_markup(sale_hunt_result_keyboard(&ctx, lang))
            .await?;
    }
    Ok(())
}

async fn send_deal_result(
    ctx: &AppContext,
    chat_id: ChatId,
    lang: &str,
    deal: &SaleHuntDeal,
    key: &str,
) -> Result<()> {
    let text = trl(
        ctx,
        lang,
        key,
        "🎉 Deal săn sale của bạn\n\nGiảm: {percent}% cho đơn tiếp theo\nMã: {code}\nHết hạn: {expires_at}\n\nDeal sẽ tự áp dụng khi bạn tạo đơn trong shop.",
        &[
            ("percent", deal.discount_percent.to_string()),
            ("code", deal.code.clone()),
            ("expires_at", deal.expires_at.clone()),
        ],
    );
    ctx.bot
        .send_message(chat_id, text)
        .reply_markup(sale_hunt_result_keyboard(ctx, lang))
        .await?;
    Ok(())
}

async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS sale_hunt_deals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            code TEXT NOT NULL UNIQUE,
            discount_percent INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            expires_at TEXT NOT NULL,
            order_id TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            used_at TEXT,
            CONSTRAINT sale_hunt_status_check CHECK (status IN ('active', 'used', 'expired'))
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sale_hunt_deals_user_status ON sale_hunt_deals (user_id, status, expires_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sale_hunt_deals_user_created ON sale_hunt_deals (user_id, created_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS sale_hunt_golden_hour_deals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            deal_date TEXT NOT NULL UNIQUE,
            starts_at TEXT NOT NULL,
            ends_at TEXT NOT NULL,
            notify_at TEXT NOT NULL,
            discount_percent INTEGER NOT NULL,
            notified_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sale_hunt_golden_hour_notify ON sale_hunt_golden_hour_deals (notify_at, notified_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sale_hunt_golden_hour_window ON sale_hunt_golden_hour_deals (starts_at, ends_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn run_golden_hour_tick(ctx: &AppContext) -> Result<()> {
    ensure_schema(&ctx.pool).await?;
    let deal = ensure_next_golden_hour(&ctx.pool).await?;
    let now = Utc::now().to_rfc3339();
    if deal.notified_at.is_some()
        || datetime_before(&now, &deal.notify_at)
        || !datetime_before(&now, &deal.starts_at)
    {
        return Ok(());
    }

    let subscribers = users_repo::list_subscribers(&ctx.pool).await?;
    let sent_at = Utc::now().to_rfc3339();
    for subscriber in subscribers {
        if subscriber.is_bot.unwrap_or(0) != 0 {
            continue;
        }
        let lang = i18n::user_lang_by_id(ctx, subscriber.user_id).await;
        let text = golden_hour_notify_text(ctx, &lang, &deal);
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                tl(ctx, &lang, "golden_hour_btn_sale_hunt", "🔥 Vào săn sale"),
                "salehunt:menu",
            ),
            InlineKeyboardButton::callback(
                tl(ctx, &lang, "start_btn_shop", "🛒 Shop"),
                "start:shop",
            ),
        ]]);
        if let Err(err) = ctx
            .bot
            .send_message(ChatId(subscriber.chat_id), text)
            .reply_markup(keyboard)
            .await
        {
            warn!(
                "failed to notify user {} about golden hour deal {}: {err}",
                subscriber.user_id, deal.id
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }

    sqlx::query("UPDATE sale_hunt_golden_hour_deals SET notified_at = ? WHERE id = ?")
        .bind(sent_at)
        .bind(deal.id)
        .execute(&ctx.pool)
        .await?;
    info!("sent golden hour deal {} notification", deal.id);
    Ok(())
}

async fn expire_old_deals(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "UPDATE sale_hunt_deals SET status = 'expired' WHERE status = 'active' AND datetime(expires_at) <= datetime(?)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

async fn count_claims_today(pool: &SqlitePool, user_id: i64) -> Result<i64> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1)
         FROM sale_hunt_deals
         WHERE user_id = ?
           AND datetime(created_at) >= datetime('now', '+7 hours', 'start of day', '-7 hours')",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

fn sale_hunt_keyboard(ctx: &AppContext, lang: &str, can_hunt: bool) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if can_hunt {
        rows.push(vec![i18n::inline_button_callback(
            ctx,
            lang,
            "sale_hunt_btn_hunt",
            "🔥 Bấm để săn sale",
            "salehunt:hunt",
        )]);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "sale_hunt_btn_my_deal",
        "🎁 Deal của tôi",
        "salehunt:mine",
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_shop",
        "🛒 Shop",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn sale_hunt_result_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_shop",
            "🛒 Shop",
            "start:shop",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "sale_hunt_btn_back",
            "⬅️ Quay lại săn sale",
            "salehunt:menu",
        )],
    ])
}

fn active_deal_line(ctx: &AppContext, lang: &str, deal: &SaleHuntDeal) -> String {
    trl(
        ctx,
        lang,
        "sale_hunt_active_deal_line",
        "Deal đang có: giảm {percent}% - mã {code} - hết hạn {expires_at}",
        &[
            ("percent", deal.discount_percent.to_string()),
            ("code", deal.code.clone()),
            ("expires_at", deal.expires_at.clone()),
        ],
    )
}

async fn ensure_next_golden_hour(pool: &SqlitePool) -> Result<GoldenHourDeal> {
    ensure_schema(pool).await?;
    let now = Utc::now().to_rfc3339();
    if let Some(deal) = sqlx::query_as::<_, GoldenHourDeal>(
        r#"SELECT id, deal_date, starts_at, ends_at, notify_at, discount_percent, notified_at, created_at
           FROM sale_hunt_golden_hour_deals
           WHERE datetime(ends_at) > datetime(?)
           ORDER BY datetime(starts_at) ASC, id ASC
           LIMIT 1"#,
    )
    .bind(&now)
    .fetch_optional(pool)
    .await?
    {
        return cap_golden_hour_deal(pool, deal).await;
    }

    let today_key = Utc::now()
        .with_timezone(&vietnam_offset())
        .format("%Y-%m-%d")
        .to_string();
    let has_today_deal = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM sale_hunt_golden_hour_deals WHERE deal_date = ?",
    )
    .bind(today_key)
    .fetch_one(pool)
    .await?
        > 0;

    let new_deal = build_random_golden_hour_deal(has_today_deal);
    sqlx::query(
        "INSERT OR IGNORE INTO sale_hunt_golden_hour_deals (deal_date, starts_at, ends_at, notify_at, discount_percent, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&new_deal.deal_date)
    .bind(&new_deal.starts_at)
    .bind(&new_deal.ends_at)
    .bind(&new_deal.notify_at)
    .bind(new_deal.discount_percent)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;

    let deal = sqlx::query_as::<_, GoldenHourDeal>(
        r#"SELECT id, deal_date, starts_at, ends_at, notify_at, discount_percent, notified_at, created_at
           FROM sale_hunt_golden_hour_deals
           WHERE deal_date = ?
           LIMIT 1"#,
    )
    .bind(new_deal.deal_date)
    .fetch_one(pool)
    .await?;
    cap_golden_hour_deal(pool, deal).await
}

async fn cap_golden_hour_deal(
    pool: &SqlitePool,
    mut deal: GoldenHourDeal,
) -> Result<GoldenHourDeal> {
    let capped = golden_hour_discount_percent(&deal);
    if deal.discount_percent != capped {
        sqlx::query("UPDATE sale_hunt_golden_hour_deals SET discount_percent = ? WHERE id = ?")
            .bind(capped)
            .bind(deal.id)
            .execute(pool)
            .await?;
        deal.discount_percent = capped;
    }
    Ok(deal)
}

struct NewGoldenHourDeal {
    deal_date: String,
    starts_at: String,
    ends_at: String,
    notify_at: String,
    discount_percent: i64,
}

fn build_random_golden_hour_deal(skip_today: bool) -> NewGoldenHourDeal {
    let offset = vietnam_offset();
    let now_local = Utc::now().with_timezone(&offset);
    let today = now_local.date_naive();
    let earliest_start = now_local + Duration::minutes(GOLDEN_HOUR_NOTIFY_BEFORE_MINUTES);
    let tomorrow = today.checked_add_signed(Duration::days(1)).unwrap_or(today);
    let start_local = if skip_today {
        random_golden_hour_start(tomorrow, None).unwrap()
    } else {
        random_golden_hour_start(today, Some(earliest_start))
            .unwrap_or_else(|| random_golden_hour_start(tomorrow, None).unwrap())
    };
    let end_local = start_local + Duration::minutes(GOLDEN_HOUR_DURATION_MINUTES);
    let notify_local = start_local - Duration::minutes(GOLDEN_HOUR_NOTIFY_BEFORE_MINUTES);
    let discount_percent =
        GOLDEN_HOUR_DISCOUNTS[rand::thread_rng().gen_range(0..GOLDEN_HOUR_DISCOUNTS.len())];

    NewGoldenHourDeal {
        deal_date: start_local.format("%Y-%m-%d").to_string(),
        starts_at: start_local.with_timezone(&Utc).to_rfc3339(),
        ends_at: end_local.with_timezone(&Utc).to_rfc3339(),
        notify_at: notify_local.with_timezone(&Utc).to_rfc3339(),
        discount_percent,
    }
}

fn random_golden_hour_start(
    date: NaiveDate,
    earliest_start: Option<DateTime<FixedOffset>>,
) -> Option<DateTime<FixedOffset>> {
    let offset = vietnam_offset();
    let slots: Vec<_> = (GOLDEN_HOUR_START_MINUTE..=GOLDEN_HOUR_END_START_MINUTE)
        .step_by(GOLDEN_HOUR_SLOT_MINUTES as usize)
        .filter_map(|minute| {
            let hour = (minute / 60) as u32;
            let minute = (minute % 60) as u32;
            let local = offset
                .from_local_datetime(&date.and_hms_opt(hour, minute, 0)?)
                .single()?;
            if earliest_start.as_ref().map_or(true, |min| local >= min.clone()) {
                Some(local)
            } else {
                None
            }
        })
        .collect();
    if slots.is_empty() {
        return None;
    }
    let index = rand::thread_rng().gen_range(0..slots.len());
    Some(slots[index])
}

fn render_golden_hour_line(ctx: &AppContext, lang: &str, deal: &GoldenHourDeal) -> String {
    trl(
        ctx,
        lang,
        "golden_hour_line",
        "⏰ Deal giờ vàng: {date_label} {start_time}-{end_time}, giảm {percent}% ({status})",
        &[
            ("date_label", golden_hour_date_label(ctx, lang, deal)),
            ("start_time", format_vietnam_hhmm(&deal.starts_at)),
            ("end_time", format_vietnam_hhmm(&deal.ends_at)),
            ("percent", golden_hour_discount_percent(deal).to_string()),
            ("status", golden_hour_status(ctx, lang, deal)),
        ],
    )
}

fn golden_hour_notify_text(ctx: &AppContext, lang: &str, deal: &GoldenHourDeal) -> String {
    trl(
        ctx,
        lang,
        "golden_hour_notify_text",
        "🔥 DEAL GIỜ VÀNG SẮP MỞ\n\nTừ {start_time} đến {end_time} {date_label}\nGiảm {percent}% toàn shop trong 30 phút.\n\nKhi thanh toán, hệ thống tự lấy deal cao nhất cho bạn.",
        &[
            ("date_label", golden_hour_date_label(ctx, lang, deal)),
            ("start_time", format_vietnam_hhmm(&deal.starts_at)),
            ("end_time", format_vietnam_hhmm(&deal.ends_at)),
            ("percent", golden_hour_discount_percent(deal).to_string()),
        ],
    )
}

fn golden_hour_status(ctx: &AppContext, lang: &str, deal: &GoldenHourDeal) -> String {
    let now = Utc::now().to_rfc3339();
    if datetime_before(&now, &deal.starts_at) {
        tl(ctx, lang, "golden_hour_status_upcoming", "sắp mở")
    } else if datetime_before(&now, &deal.ends_at) {
        tl(ctx, lang, "golden_hour_status_active", "đang mở")
    } else {
        tl(ctx, lang, "golden_hour_status_ended", "đã kết thúc")
    }
}

fn golden_hour_date_label(ctx: &AppContext, lang: &str, deal: &GoldenHourDeal) -> String {
    let offset = vietnam_offset();
    let today = Utc::now().with_timezone(&offset).format("%Y-%m-%d").to_string();
    let tomorrow = (Utc::now().with_timezone(&offset) + Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    if deal.deal_date == today {
        tl(ctx, lang, "golden_hour_date_today", "hôm nay")
    } else if deal.deal_date == tomorrow {
        tl(ctx, lang, "golden_hour_date_tomorrow", "ngày mai")
    } else {
        deal.deal_date.clone()
    }
}

fn format_vietnam_hhmm(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| {
            dt.with_timezone(&vietnam_offset())
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn datetime_before(left: &str, right: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(left),
        DateTime::parse_from_rfc3339(right),
    ) {
        (Ok(left), Ok(right)) => left < right,
        _ => left < right,
    }
}

fn datetime_before_or_equal(left: &str, right: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(left),
        DateTime::parse_from_rfc3339(right),
    ) {
        (Ok(left), Ok(right)) => left <= right,
        _ => left <= right,
    }
}

fn vietnam_offset() -> FixedOffset {
    FixedOffset::east_opt(7 * 60 * 60).expect("Vietnam timezone offset is valid")
}

fn random_discount_percent() -> i64 {
    let index = rand::thread_rng().gen_range(0..DISCOUNT_CHOICES.len());
    DISCOUNT_CHOICES[index]
}

fn generate_code(percent: i64) -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(4)
        .collect::<String>()
        .to_uppercase();
    format!("HUNT{percent}-{suffix}")
}

fn is_sale_hunt_text(ctx: &AppContext, text: &str) -> bool {
    if text.eq_ignore_ascii_case("/sale") || text.eq_ignore_ascii_case("/salehunt") {
        return true;
    }
    let input = i18n::button_text_match_variants(text);
    ctx.texts
        .read()
        .map(|texts| {
            texts
                .enabled_languages()
                .into_iter()
                .any(|language| {
                    let label = texts.get_lang("start_btn_sale_hunt", &language.code, "🔥 Săn sale");
                    let variants = i18n::button_text_match_variants(&label);
                    variants
                        .iter()
                        .any(|variant| input.iter().any(|value| value == variant))
                })
        })
        .unwrap_or(false)
}

fn tl(ctx: &AppContext, lang: &str, key: &str, default: &str) -> String {
    i18n::t(ctx, lang, key, default)
}

fn trl(ctx: &AppContext, lang: &str, key: &str, default: &str, vars: &[(&str, String)]) -> String {
    i18n::tr(ctx, lang, key, default, vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discount_amount_uses_percent_and_minimum_one() {
        assert_eq!(discount_amount(100_000, 10), 10_000);
        assert_eq!(discount_amount(9, 5), 1);
        assert_eq!(discount_amount(100_000, 0), 0);
    }
}
