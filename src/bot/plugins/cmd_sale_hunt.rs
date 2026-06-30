use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, Utc};
use rand::{Rng, distributions::Alphanumeric};
use serde::Serialize;
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, CallbackQuery, ChatId, InlineKeyboardMarkup, Message};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State, i18n};

const DEAL_TTL_MINUTES: i64 = 30;
const DAILY_CLAIM_LIMIT: i64 = 1;
const DISCOUNT_CHOICES: [i64; 8] = [5, 5, 7, 7, 10, 10, 12, 15];

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
    let text = trl(
        &ctx,
        lang,
        "sale_hunt_menu_text",
        "🔥 SĂN SALE HÔM NAY\n\nMỗi tài khoản săn được 1 deal/ngày. Deal áp dụng tự động cho đơn tiếp theo và hết hạn sau {ttl} phút.\n\n{deal_line}\n\nLượt hôm nay: {used}/{limit}",
        &[
            ("ttl", DEAL_TTL_MINUTES.to_string()),
            ("deal_line", deal_line),
            ("used", claims_today.min(DAILY_CLAIM_LIMIT).to_string()),
            ("limit", DAILY_CLAIM_LIMIT.to_string()),
        ],
    );
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
