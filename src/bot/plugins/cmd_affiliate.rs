use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use sqlx::{FromRow, SqlitePool};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message};
use tokio::time::{Duration, sleep};
use url::Url;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, i18n};
use crate::domains::orders::models::{Order, OrderStatus};
use crate::domains::orders::repo as orders_repo;
use crate::domains::products::models::Product;

pub struct AffiliateCommandPlugin;

const DEFAULT_COMMISSION_BPS: i64 = 1_000; // 10%
const MAX_COMMISSION_BPS: i64 = 10_000;

#[derive(Debug, Clone, FromRow)]
struct AffiliatePartner {
    user_id: i64,
    code: String,
    commission_bps: i64,
}

#[derive(Debug, Clone, FromRow)]
struct AffiliateStats {
    referral_count: i64,
    order_count: i64,
    pending_commission: i64,
    total_commission: i64,
}

#[derive(Debug, Clone, FromRow)]
struct AffiliateListRow {
    user_id: i64,
    code: String,
    commission_bps: i64,
    referral_count: i64,
    order_count: i64,
    total_commission: i64,
}

#[async_trait::async_trait]
impl AppPlugin for AffiliateCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdAffiliate"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_affiliate_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![
            BotCommand {
                command: "ctv".to_string(),
                description: "Affiliate dashboard".to_string(),
            },
            BotCommand {
                command: "ctvadd".to_string(),
                description: "Admin: add affiliate".to_string(),
            },
            BotCommand {
                command: "ctvoff".to_string(),
                description: "Admin: disable affiliate".to_string(),
            },
            BotCommand {
                command: "ctvlist".to_string(),
                description: "Admin: list affiliates".to_string(),
            },
        ]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();

        if let Some(ref_code) = start_ref_code(text) {
            handle_start_ref(&ctx, &msg, &ref_code).await?;
            return Ok(true);
        }

        if is_command(text, "/ctv") {
            handle_ctv_dashboard(&ctx, &msg).await?;
            return Ok(true);
        }

        if is_command(text, "/ctvadd") {
            handle_ctvadd(&ctx, &msg, text).await?;
            return Ok(true);
        }

        if is_command(text, "/ctvoff") {
            handle_ctvoff(&ctx, &msg, text).await?;
            return Ok(true);
        }

        if is_command(text, "/ctvlist") {
            handle_ctvlist(&ctx, &msg).await?;
            return Ok(true);
        }

        Ok(false)
    }

    async fn on_order_paid(
        &self,
        ctx: Arc<AppContext>,
        order: &Order,
        product: &Product,
    ) -> Result<Option<String>, anyhow::Error> {
        let order_snapshot = order.clone();
        let product_id = product.id;
        let product_name = product.name.clone();
        tokio::spawn(async move {
            if let Err(err) = record_commission_when_order_is_paid(
                ctx,
                order_snapshot,
                product_id,
                product_name,
            )
            .await
            {
                tracing::warn!("affiliate commission hook failed: {err}");
            }
        });
        Ok(None)
    }
}

async fn ensure_affiliate_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS affiliate_partners (
            user_id INTEGER PRIMARY KEY,
            code TEXT NOT NULL UNIQUE,
            commission_bps INTEGER NOT NULL DEFAULT 1000,
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS affiliate_referrals (
            referred_user_id INTEGER PRIMARY KEY,
            affiliate_user_id INTEGER NOT NULL,
            ref_code TEXT NOT NULL,
            first_order_id TEXT,
            first_paid_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS affiliate_commissions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            affiliate_user_id INTEGER NOT NULL,
            referred_user_id INTEGER NOT NULL,
            order_id TEXT NOT NULL UNIQUE,
            product_id INTEGER NOT NULL,
            amount INTEGER NOT NULL,
            commission_amount INTEGER NOT NULL,
            commission_bps INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_affiliate_referrals_affiliate ON affiliate_referrals(affiliate_user_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_affiliate_commissions_affiliate ON affiliate_commissions(affiliate_user_id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

fn start_ref_code(text: &str) -> Option<String> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;
    if !command_name_matches(command, "/start") {
        return None;
    }
    let payload = parts.next()?.trim();
    payload
        .strip_prefix("ref_")
        .or_else(|| payload.strip_prefix("ref-"))
        .or_else(|| payload.strip_prefix("ctv_"))
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string)
}

async fn handle_start_ref(ctx: &AppContext, msg: &Message, ref_code: &str) -> Result<()> {
    let Some(user) = msg.from() else {
        return Ok(());
    };
    ensure_affiliate_schema(&ctx.pool).await?;

    let user_id = user.id.0 as i64;
    let lang = i18n::user_lang(ctx, user_id, user.language_code.as_deref()).await;
    let Some(partner) = get_active_partner_by_code(&ctx.pool, ref_code).await? else {
        ctx.bot
            .send_message(
                msg.chat.id,
                i18n::t(
                    ctx,
                    &lang,
                    "affiliate_ref_invalid",
                    "Link CTV không hợp lệ hoặc đã tắt.",
                ),
            )
            .await?;
        return Ok(());
    };

    if partner.user_id == user_id {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không thể tự giới thiệu chính mình.")
            .await?;
        return Ok(());
    }

    let inserted = insert_referral_once(&ctx.pool, user_id, partner.user_id, &partner.code).await?;
    let text = if inserted {
        "✅ Đã ghi nhận link giới thiệu CTV. Bạn có thể bắt đầu mua hàng trong bot."
    } else {
        "✅ Bạn đã được ghi nhận nguồn giới thiệu trước đó. Mời bạn tiếp tục mua hàng."
    };

    ctx.bot
        .send_message(msg.chat.id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "🛒 Xem sản phẩm",
            "start:shop",
        )]]))
        .await?;
    Ok(())
}

async fn handle_ctv_dashboard(ctx: &AppContext, msg: &Message) -> Result<()> {
    let Some(user) = msg.from() else {
        return Ok(());
    };
    ensure_affiliate_schema(&ctx.pool).await?;

    let user_id = user.id.0 as i64;
    let Some(partner) = get_partner_by_user_id(&ctx.pool, user_id).await? else {
        let mut text = "Bạn chưa được bật quyền CTV. Hãy liên hệ admin để được cấp link giới thiệu.".to_string();
        if is_affiliate_admin(ctx, user_id) {
            text.push_str("\n\nAdmin dùng: /ctvadd <telegram_id> [hoa_hong_%]\nVí dụ: /ctvadd 5919002786 10");
        }
        ctx.bot.send_message(msg.chat.id, text).await?;
        return Ok(());
    };

    let stats = affiliate_stats(&ctx.pool, user_id).await?;
    let url = affiliate_url(ctx, &partner.code).await?;
    let text = format!(
        "🤝 CTV của bạn\n\nLink giới thiệu:\n{}\n\nHoa hồng: {}\nKhách đã giới thiệu: {}\nĐơn có hoa hồng: {}\nHoa hồng chờ rút: {}\nTổng hoa hồng: {}",
        url,
        format_percent_bps(partner.commission_bps),
        stats.referral_count,
        stats.order_count,
        format_vnd(stats.pending_commission),
        format_vnd(stats.total_commission),
    );

    ctx.bot
        .send_message(msg.chat.id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "🔗 Mở link CTV",
            Url::parse(&url)?,
        )]]))
        .await?;
    Ok(())
}

async fn handle_ctvadd(ctx: &AppContext, msg: &Message, text: &str) -> Result<()> {
    let Some(admin) = msg.from() else {
        return Ok(());
    };
    if !is_affiliate_admin(ctx, admin.id.0 as i64) {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không có quyền quản lý CTV.")
            .await?;
        return Ok(());
    }

    let mut parts = text.split_whitespace();
    let _ = parts.next();
    let Some(user_id) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
        ctx.bot
            .send_message(msg.chat.id, "Cách dùng: /ctvadd <telegram_id> [hoa_hong_%]\nVí dụ: /ctvadd 5919002786 10")
            .await?;
        return Ok(());
    };
    let commission_bps = parts
        .next()
        .and_then(percent_to_bps)
        .unwrap_or(DEFAULT_COMMISSION_BPS);

    ensure_affiliate_schema(&ctx.pool).await?;
    let partner = upsert_partner(&ctx.pool, user_id, commission_bps).await?;
    let url = affiliate_url(ctx, &partner.code).await?;
    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "✅ Đã bật CTV\nUser ID: {}\nHoa hồng: {}\nLink: {}",
                partner.user_id,
                format_percent_bps(partner.commission_bps),
                url
            ),
        )
        .await?;
    Ok(())
}

async fn handle_ctvoff(ctx: &AppContext, msg: &Message, text: &str) -> Result<()> {
    let Some(admin) = msg.from() else {
        return Ok(());
    };
    if !is_affiliate_admin(ctx, admin.id.0 as i64) {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không có quyền quản lý CTV.")
            .await?;
        return Ok(());
    }

    let Some(user_id) = text
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<i64>().ok())
    else {
        ctx.bot
            .send_message(msg.chat.id, "Cách dùng: /ctvoff <telegram_id>")
            .await?;
        return Ok(());
    };

    ensure_affiliate_schema(&ctx.pool).await?;
    let affected = sqlx::query(
        "UPDATE affiliate_partners SET is_active = 0, updated_at = ? WHERE user_id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(user_id)
    .execute(&ctx.pool)
    .await?
    .rows_affected();

    let text = if affected > 0 {
        format!("✅ Đã tắt CTV user ID {user_id}.")
    } else {
        format!("Không tìm thấy CTV user ID {user_id}.")
    };
    ctx.bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

async fn handle_ctvlist(ctx: &AppContext, msg: &Message) -> Result<()> {
    let Some(admin) = msg.from() else {
        return Ok(());
    };
    if !is_affiliate_admin(ctx, admin.id.0 as i64) {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không có quyền xem danh sách CTV.")
            .await?;
        return Ok(());
    }

    ensure_affiliate_schema(&ctx.pool).await?;
    let rows = sqlx::query_as::<_, AffiliateListRow>(
        r#"SELECT
            p.user_id,
            p.code,
            p.commission_bps,
            COUNT(DISTINCT r.referred_user_id) AS referral_count,
            COUNT(DISTINCT c.order_id) AS order_count,
            COALESCE(SUM(c.commission_amount), 0) AS total_commission
        FROM affiliate_partners p
        LEFT JOIN affiliate_referrals r ON r.affiliate_user_id = p.user_id
        LEFT JOIN affiliate_commissions c ON c.affiliate_user_id = p.user_id
        WHERE p.is_active = 1
        GROUP BY p.user_id, p.code, p.commission_bps
        ORDER BY total_commission DESC, referral_count DESC
        LIMIT 20"#,
    )
    .fetch_all(&ctx.pool)
    .await?;

    if rows.is_empty() {
        ctx.bot.send_message(msg.chat.id, "Chưa có CTV nào.").await?;
        return Ok(());
    }

    let mut lines = vec!["👥 Danh sách CTV".to_string(), String::new()];
    for row in rows {
        lines.push(format!(
            "{} | {} | ref: {} | đơn: {} | tổng: {}",
            row.user_id,
            format_percent_bps(row.commission_bps),
            row.referral_count,
            row.order_count,
            format_vnd(row.total_commission),
        ));
    }
    ctx.bot.send_message(msg.chat.id, lines.join("\n")).await?;
    Ok(())
}

async fn record_commission_when_order_is_paid(
    ctx: Arc<AppContext>,
    order: Order,
    product_id: i64,
    product_name: String,
) -> Result<()> {
    for _ in 0..20 {
        sleep(Duration::from_millis(500)).await;
        let Some(saved_order) = orders_repo::get_order(&ctx.pool, &order.id).await? else {
            continue;
        };
        if matches!(saved_order.status, OrderStatus::Paid) {
            record_paid_order_commission(&ctx, &saved_order, product_id, &product_name).await?;
            return Ok(());
        }
        if !matches!(saved_order.status, OrderStatus::Pending) {
            return Ok(());
        }
    }
    Ok(())
}

async fn record_paid_order_commission(
    ctx: &AppContext,
    order: &Order,
    product_id: i64,
    product_name: &str,
) -> Result<()> {
    ensure_affiliate_schema(&ctx.pool).await?;
    let Some(partner) = affiliate_for_referred_user(&ctx.pool, order.user_id).await? else {
        return Ok(());
    };

    if partner.user_id == order.user_id {
        return Ok(());
    }

    let commission_amount = order.amount.saturating_mul(partner.commission_bps) / 10_000;
    if commission_amount <= 0 {
        return Ok(());
    }

    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO affiliate_commissions
        (affiliate_user_id, referred_user_id, order_id, product_id, amount, commission_amount, commission_bps, status)
        VALUES (?, ?, ?, ?, ?, ?, ?, 'pending')"#,
    )
    .bind(partner.user_id)
    .bind(order.user_id)
    .bind(&order.id)
    .bind(product_id)
    .bind(order.amount)
    .bind(commission_amount)
    .bind(partner.commission_bps)
    .execute(&ctx.pool)
    .await?;

    sqlx::query(
        "UPDATE affiliate_referrals SET first_order_id = COALESCE(first_order_id, ?), first_paid_at = COALESCE(first_paid_at, ?) WHERE referred_user_id = ?",
    )
    .bind(&order.id)
    .bind(order.paid_at.as_deref().unwrap_or(""))
    .bind(order.user_id)
    .execute(&ctx.pool)
    .await?;

    if result.rows_affected() > 0 {
        let text = format!(
            "🎉 Bạn có hoa hồng CTV mới\n\nSản phẩm: {}\nĐơn: {}\nDoanh thu: {}\nHoa hồng: {}\n\nGõ /ctv để xem thống kê.",
            product_name,
            order.bank_memo,
            format_vnd(order.amount),
            format_vnd(commission_amount),
        );
        let _ = ctx.bot.send_message(ChatId(partner.user_id), text).await;
    }

    Ok(())
}

async fn get_active_partner_by_code(pool: &SqlitePool, code: &str) -> Result<Option<AffiliatePartner>> {
    let partner = sqlx::query_as::<_, AffiliatePartner>(
        "SELECT user_id, code, commission_bps FROM affiliate_partners WHERE code = ? AND is_active = 1",
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;
    Ok(partner)
}

async fn get_partner_by_user_id(pool: &SqlitePool, user_id: i64) -> Result<Option<AffiliatePartner>> {
    let partner = sqlx::query_as::<_, AffiliatePartner>(
        "SELECT user_id, code, commission_bps FROM affiliate_partners WHERE user_id = ? AND is_active = 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(partner)
}

async fn affiliate_for_referred_user(pool: &SqlitePool, referred_user_id: i64) -> Result<Option<AffiliatePartner>> {
    let partner = sqlx::query_as::<_, AffiliatePartner>(
        r#"SELECT p.user_id, p.code, p.commission_bps
        FROM affiliate_referrals r
        JOIN affiliate_partners p ON p.user_id = r.affiliate_user_id
        WHERE r.referred_user_id = ? AND p.is_active = 1"#,
    )
    .bind(referred_user_id)
    .fetch_optional(pool)
    .await?;
    Ok(partner)
}

async fn insert_referral_once(
    pool: &SqlitePool,
    referred_user_id: i64,
    affiliate_user_id: i64,
    ref_code: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO affiliate_referrals (referred_user_id, affiliate_user_id, ref_code) VALUES (?, ?, ?)",
    )
    .bind(referred_user_id)
    .bind(affiliate_user_id)
    .bind(ref_code)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn upsert_partner(pool: &SqlitePool, user_id: i64, commission_bps: i64) -> Result<AffiliatePartner> {
    let commission_bps = commission_bps.clamp(1, MAX_COMMISSION_BPS);
    let code = affiliate_code_for_user(user_id);
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO affiliate_partners (user_id, code, commission_bps, is_active, created_at, updated_at)
        VALUES (?, ?, ?, 1, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET
            commission_bps = excluded.commission_bps,
            is_active = 1,
            updated_at = excluded.updated_at"#,
    )
    .bind(user_id)
    .bind(&code)
    .bind(commission_bps)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(AffiliatePartner {
        user_id,
        code,
        commission_bps,
    })
}

async fn affiliate_stats(pool: &SqlitePool, affiliate_user_id: i64) -> Result<AffiliateStats> {
    let stats = sqlx::query_as::<_, AffiliateStats>(
        r#"SELECT
            (SELECT COUNT(1) FROM affiliate_referrals WHERE affiliate_user_id = ?) AS referral_count,
            (SELECT COUNT(1) FROM affiliate_commissions WHERE affiliate_user_id = ?) AS order_count,
            (SELECT COALESCE(SUM(commission_amount), 0) FROM affiliate_commissions WHERE affiliate_user_id = ? AND status = 'pending') AS pending_commission,
            (SELECT COALESCE(SUM(commission_amount), 0) FROM affiliate_commissions WHERE affiliate_user_id = ?) AS total_commission"#,
    )
    .bind(affiliate_user_id)
    .bind(affiliate_user_id)
    .bind(affiliate_user_id)
    .bind(affiliate_user_id)
    .fetch_one(pool)
    .await?;
    Ok(stats)
}

async fn affiliate_url(ctx: &AppContext, code: &str) -> Result<String> {
    let me = ctx.bot.get_me().await?;
    let username = me.user.username.unwrap_or_default();
    Ok(format!("https://t.me/{username}?start=ref_{code}"))
}

fn affiliate_code_for_user(user_id: i64) -> String {
    format!("u{user_id}")
}

fn is_affiliate_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}

fn is_command(text: &str, command: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    command_name_matches(first, command)
}

fn command_name_matches(text: &str, command: &str) -> bool {
    text == command || text.starts_with(&format!("{command}@"))
}

fn percent_to_bps(value: &str) -> Option<i64> {
    let value = value.trim().trim_end_matches('%');
    if value.is_empty() {
        return None;
    }
    let mut parts = value.splitn(2, '.');
    let whole = parts.next()?.parse::<i64>().ok()?;
    let fraction = parts
        .next()
        .unwrap_or("")
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(2)
        .collect::<String>();
    let fraction = format!("{fraction:0<2}")
        .chars()
        .take(2)
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    Some((whole * 100 + fraction).clamp(1, MAX_COMMISSION_BPS))
}

fn format_percent_bps(bps: i64) -> String {
    let whole = bps / 100;
    let fraction = bps % 100;
    if fraction == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{fraction:02}%")
    }
}

fn format_vnd(amount: i64) -> String {
    let raw = amount.abs().to_string();
    let mut grouped = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    let mut value = grouped.chars().rev().collect::<String>();
    if amount < 0 {
        value.insert(0, '-');
    }
    format!("{value}đ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_ref_code_accepts_ref_payloads() {
        assert_eq!(start_ref_code("/start ref_u42").as_deref(), Some("u42"));
        assert_eq!(start_ref_code("/start@shopbot ctv_u42").as_deref(), Some("u42"));
        assert_eq!(start_ref_code("/start shop"), None);
    }

    #[test]
    fn percent_to_bps_parses_common_admin_inputs() {
        assert_eq!(percent_to_bps("10"), Some(1000));
        assert_eq!(percent_to_bps("7.5%"), Some(750));
        assert_eq!(percent_to_bps("100"), Some(10000));
    }
}
