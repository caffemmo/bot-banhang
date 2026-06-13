use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;
use rand::{Rng, distributions::Alphanumeric};
use sqlx::{Sqlite, Transaction};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, CallbackQuery, ChatId, Message};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_wallet::format_vnd;
use crate::bot::BotDialogue;
use crate::domains::childbot::repo as childbot_repo;
use crate::domains::orders::admin_notify::notify_admins_order_paid;
use crate::domains::orders::api as orders_api;
use crate::domains::orders::fulfillment::PaymentSource;
use crate::domains::orders::models::{Order, OrderStatus, OrderWithProduct};
use crate::domains::orders::repo as orders_repo;
use crate::domains::products::models::{Product, ProductPlan};
use crate::domains::products::repo as products_repo;
use crate::domains::wallet::repo as wallet_repo;

pub struct ChildBotCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for ChildBotCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdChildBot"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        childbot_repo::ensure_schema(pool).await
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![
            BotCommand {
                command: "childbotadd".to_string(),
                description: "Admin: create child bot API key".to_string(),
            },
            BotCommand {
                command: "childbotlist".to_string(),
                description: "Admin: list child bots".to_string(),
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
        if is_command(text, "/childbotadd") {
            handle_childbotadd(&ctx, &msg, text).await?;
            return Ok(true);
        }
        if is_command(text, "/childbotlist") {
            handle_childbotlist(&ctx, &msg).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(data) = q.data.clone() else {
            return Ok(false);
        };
        if let Some((action, request_id, token)) = parse_childbot_order_callback(&data) {
            match action {
                "confirm" => confirm_childbot_order(&ctx, &q, request_id, &token).await?,
                "cancel" => cancel_childbot_order(&ctx, &q, request_id, &token).await?,
                _ => return Ok(false),
            }
            return Ok(true);
        }
        Ok(false)
    }
}

async fn handle_childbotadd(ctx: &AppContext, msg: &Message, text: &str) -> Result<()> {
    let Some(admin) = msg.from() else {
        return Ok(());
    };
    if !is_childbot_admin(ctx, admin.id.0 as i64) {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không có quyền tạo bot con.")
            .await?;
        return Ok(());
    }

    let mut parts = text.split_whitespace();
    let _ = parts.next();
    let Some(owner_user_id) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
        ctx.bot
            .send_message(
                msg.chat.id,
                "Cách dùng: /childbotadd <telegram_id_ctv> [@bot_username] [tên shop]",
            )
            .await?;
        return Ok(());
    };
    let bot_username = parts.next();
    let shop_name = parts.collect::<Vec<_>>().join(" ");
    let shop_name = if shop_name.trim().is_empty() {
        None
    } else {
        Some(shop_name.as_str())
    };

    let (record, token) = childbot_repo::create_child_bot(
        &ctx.pool,
        owner_user_id,
        bot_username,
        shop_name,
    )
    .await?;
    let base_url = ctx
        .base_url()
        .unwrap_or_else(|| "https://caffemmo.com".to_string());
    let text = format!(
        "✅ Đã tạo API key bot con\n\nChild bot ID: {}\nCTV Telegram ID: {}\nBot username: {}\nTên shop: {}\n\nAPI_BASE_URL={}\nCHILDBOT_API_KEY={}\n\nLưu ý: key này chỉ tạo yêu cầu mua, không có quyền trừ ví trực tiếp.",
        record.id,
        record.owner_user_id,
        record.bot_username.clone().unwrap_or_else(|| "-".to_string()),
        record.shop_name.clone().unwrap_or_else(|| "-".to_string()),
        base_url,
        token,
    );
    ctx.bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

async fn handle_childbotlist(ctx: &AppContext, msg: &Message) -> Result<()> {
    let Some(admin) = msg.from() else {
        return Ok(());
    };
    if !is_childbot_admin(ctx, admin.id.0 as i64) {
        ctx.bot
            .send_message(msg.chat.id, "Bạn không có quyền xem bot con.")
            .await?;
        return Ok(());
    }
    let rows = childbot_repo::list_child_bots(&ctx.pool, 20).await?;
    if rows.is_empty() {
        ctx.bot.send_message(msg.chat.id, "Chưa có bot con nào.").await?;
        return Ok(());
    }
    let mut lines = vec!["🤖 Danh sách bot con".to_string(), String::new()];
    for row in rows {
        lines.push(format!(
            "#{} | CTV {} | {} | {} | active={}",
            row.id,
            row.owner_user_id,
            row.bot_username.unwrap_or_else(|| "-".to_string()),
            row.shop_name.unwrap_or_else(|| "-".to_string()),
            row.is_active,
        ));
    }
    ctx.bot.send_message(msg.chat.id, lines.join("\n")).await?;
    Ok(())
}

async fn confirm_childbot_order(
    ctx: &AppContext,
    q: &CallbackQuery,
    request_id: i64,
    token: &str,
) -> Result<()> {
    let Some(request) = childbot_repo::get_purchase_request(&ctx.pool, request_id).await? else {
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Yêu cầu mua không tồn tại.")
            .await;
        return Ok(());
    };
    if request.confirm_token != token || request.buyer_user_id != q.from.id.0 as i64 {
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Yêu cầu mua không hợp lệ.")
            .await;
        return Ok(());
    }
    if request.status != "pending" {
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Yêu cầu này đã được xử lý.")
            .await;
        return Ok(());
    }

    let _ = ctx
        .bot
        .answer_callback_query(q.id.clone())
        .text("Đang xử lý đơn...")
        .await;

    match fulfill_childbot_request(ctx, &request).await {
        Ok(response) => {
            if let Some(msg) = &q.message {
                ctx.bot
                    .send_message(
                        msg.chat().id,
                        format!(
                            "✅ Mua hàng thành công\n\nĐơn: {}\nSố tiền: {}\nSố dư còn lại: {}\n\nDữ liệu giao hàng:\n{}",
                            response.order_id,
                            format_vnd(response.amount),
                            format_vnd(response.balance_after),
                            response.delivered_data,
                        ),
                    )
                    .await?;
            }
        }
        Err(err) => {
            if let Some(msg) = &q.message {
                ctx.bot
                    .send_message(msg.chat().id, format!("❌ Không thể mua hàng: {err}"))
                    .await?;
            }
        }
    }
    Ok(())
}

async fn cancel_childbot_order(
    ctx: &AppContext,
    q: &CallbackQuery,
    request_id: i64,
    token: &str,
) -> Result<()> {
    let Some(request) = childbot_repo::get_purchase_request(&ctx.pool, request_id).await? else {
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        return Ok(());
    };
    if request.confirm_token == token && request.buyer_user_id == q.from.id.0 as i64 {
        let _ = childbot_repo::mark_purchase_request_status(&ctx.pool, request_id, "cancelled", None).await;
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Đã hủy yêu cầu mua.")
            .await;
        if let Some(msg) = &q.message {
            let _ = ctx.bot.send_message(msg.chat().id, "Đã hủy yêu cầu mua từ bot con.").await;
        }
    }
    Ok(())
}

struct FulfilledChildBotOrder {
    order_id: String,
    amount: i64,
    balance_after: i64,
    delivered_data: String,
}

async fn fulfill_childbot_request(
    ctx: &AppContext,
    request: &childbot_repo::ChildBotPurchaseRequest,
) -> Result<FulfilledChildBotOrder> {
    let product = products_repo::get_product(&ctx.pool, request.product_id)
        .await?
        .filter(|p| p.is_active.unwrap_or(1) == 1)
        .ok_or_else(|| anyhow!("product not found"))?;
    let delivery_type = orders_api::product_delivery_type(&product).to_string();
    let plans = products_repo::list_product_plans(&ctx.pool, product.id).await?;
    let selected_plan = select_plan(request.plan_id, &plans, &delivery_type)?;
    let amount = selected_plan
        .as_ref()
        .map(|plan| plan.price)
        .unwrap_or(product.price * request.qty);
    if amount != request.amount {
        return Err(anyhow!("Giá sản phẩm đã thay đổi, vui lòng tạo yêu cầu mua mới."));
    }

    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, request.buyer_user_id).await?;
    if wallet.balance < amount {
        return Err(anyhow!(
            "Số dư ví không đủ. Hiện có {}, cần {}",
            format_vnd(wallet.balance),
            format_vnd(amount),
        ));
    }

    let mut order = Order::new(
        request.buyer_user_id,
        request.buyer_chat_id,
        product.id,
        request.qty,
        amount,
        new_childbot_memo(request.child_bot_id),
        request.customer_input.clone(),
        selected_plan.as_ref().map(|plan| plan.id),
        selected_plan.as_ref().map(|plan| plan.label.clone()),
        selected_plan.as_ref().map(|plan| plan.months),
        selected_plan.as_ref().map(|plan| plan.price),
    );

    let mut tx = ctx.pool.begin().await?;
    let (delivered_data, reserved_item_ids) =
        reserve_delivery_data(&mut tx, &product, &delivery_type, request.qty, request.customer_input.as_deref()).await?;
    order.delivered_data = Some(delivered_data.clone());
    order.reserved_item_ids = reserved_item_ids;
    orders_repo::insert_order_tx(&mut tx, &order).await?;
    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        request.buyer_user_id,
        amount,
        &order.id,
        Some("childbot_wallet_purchase"),
    )
    .await?;
    let paid_at = Utc::now();
    orders_repo::mark_order_paid(
        &mut tx,
        &order.id,
        "childbot_wallet",
        paid_at,
        Some(&delivered_data),
        order.reserved_item_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    childbot_repo::mark_purchase_request_status(&ctx.pool, request.id, "paid", Some(&order.id)).await?;
    childbot_repo::insert_child_bot_order(
        &ctx.pool,
        &order.id,
        request.child_bot_id,
        request.affiliate_user_id,
        request.buyer_user_id,
    )
    .await?;
    record_childbot_commission(ctx, request.affiliate_user_id, request.buyer_user_id, &order, &product).await?;

    order.status = OrderStatus::Paid;
    order.payment_tx_id = Some("childbot_wallet".to_string());
    order.paid_at = Some(paid_at.to_rfc3339());
    let paid_order = OrderWithProduct {
        order: order.clone(),
        product: product.clone(),
    };
    if let Err(err) = notify_admins_order_paid(
        ctx,
        &paid_order,
        "childbot_wallet",
        paid_at,
        &PaymentSource::ClientApiWallet,
    )
    .await
    {
        tracing::error!("send paid-order admin notification after child bot wallet payment failed: {err}");
    }

    Ok(FulfilledChildBotOrder {
        order_id: order.id,
        amount,
        balance_after,
        delivered_data,
    })
}

fn select_plan<'a>(
    plan_id: Option<i64>,
    plans: &'a [ProductPlan],
    delivery_type: &str,
) -> Result<Option<&'a ProductPlan>> {
    if delivery_type != "manual_input" {
        return Ok(None);
    }
    if plans.is_empty() {
        return Ok(None);
    }
    let plan_id = plan_id.ok_or_else(|| anyhow!("plan_id is required for this product"))?;
    plans
        .iter()
        .find(|plan| plan.id == plan_id)
        .map(Some)
        .ok_or_else(|| anyhow!("plan not found"))
}

async fn reserve_delivery_data(
    tx: &mut Transaction<'_, sqlx::Sqlite>,
    product: &Product,
    delivery_type: &str,
    qty: i64,
    customer_input: Option<&str>,
) -> Result<(String, Option<String>)> {
    if delivery_type == "manual_input" {
        let info = customer_input
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("N/A");
        return Ok((format!("info: {info}"), None));
    }

    let reserved = products_repo::take_product_items(tx, product.id, qty).await?;
    let data = reserved
        .iter()
        .map(|i| i.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim().is_empty() {
        return Err(anyhow!("stock is empty"));
    }
    let reserved_ids = reserved
        .iter()
        .map(|item| item.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    Ok((data, Some(reserved_ids)))
}

async fn record_childbot_commission(
    ctx: &AppContext,
    affiliate_user_id: i64,
    buyer_user_id: i64,
    order: &Order,
    product: &Product,
) -> Result<()> {
    if affiliate_user_id == buyer_user_id {
        return Ok(());
    }
    ensure_affiliate_schema(&ctx.pool).await?;
    let code = format!("u{affiliate_user_id}");
    sqlx::query(
        r#"INSERT OR IGNORE INTO affiliate_partners
        (user_id, code, commission_bps, is_active, created_at, updated_at)
        VALUES (?, ?, 500, 1, datetime('now'), datetime('now'))"#,
    )
    .bind(affiliate_user_id)
    .bind(&code)
    .execute(&ctx.pool)
    .await?;

    let commission_bps: i64 = sqlx::query_scalar(
        "SELECT commission_bps FROM affiliate_partners WHERE user_id = ? AND is_active = 1",
    )
    .bind(affiliate_user_id)
    .fetch_optional(&ctx.pool)
    .await?
    .unwrap_or(500);
    let commission_amount = order.amount.saturating_mul(commission_bps) / 10_000;
    if commission_amount <= 0 {
        return Ok(());
    }

    sqlx::query(
        "INSERT OR IGNORE INTO affiliate_referrals (referred_user_id, affiliate_user_id, ref_code) VALUES (?, ?, ?)",
    )
    .bind(buyer_user_id)
    .bind(affiliate_user_id)
    .bind(&code)
    .execute(&ctx.pool)
    .await?;

    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO affiliate_commissions
        (affiliate_user_id, referred_user_id, order_id, product_id, amount, commission_amount, commission_bps, status)
        VALUES (?, ?, ?, ?, ?, ?, ?, 'pending')"#,
    )
    .bind(affiliate_user_id)
    .bind(buyer_user_id)
    .bind(&order.id)
    .bind(product.id)
    .bind(order.amount)
    .bind(commission_amount)
    .bind(commission_bps)
    .execute(&ctx.pool)
    .await?;

    sqlx::query(
        "UPDATE affiliate_referrals SET first_order_id = COALESCE(first_order_id, ?), first_paid_at = COALESCE(first_paid_at, ?) WHERE referred_user_id = ?",
    )
    .bind(&order.id)
    .bind(order.paid_at.as_deref().unwrap_or(""))
    .bind(buyer_user_id)
    .execute(&ctx.pool)
    .await?;

    if result.rows_affected() > 0 {
        let text = format!(
            "🎉 Bạn có hoa hồng bot con mới\n\nSản phẩm: {}\nĐơn: {}\nDoanh thu: {}\nHoa hồng: {}\n\nGõ /ctv để xem thống kê.",
            product.name,
            order.bank_memo,
            format_vnd(order.amount),
            format_vnd(commission_amount),
        );
        let _ = ctx.bot.send_message(ChatId(affiliate_user_id), text).await;
    }

    Ok(())
}

async fn ensure_affiliate_schema(pool: &sqlx::SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS affiliate_partners (
            user_id INTEGER PRIMARY KEY,
            code TEXT NOT NULL UNIQUE,
            commission_bps INTEGER NOT NULL DEFAULT 500,
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
    Ok(())
}

fn parse_childbot_order_callback(data: &str) -> Option<(&str, i64, String)> {
    let mut parts = data.split(':');
    if parts.next()? != "childbot_order" {
        return None;
    }
    let action = parts.next()?;
    let request_id = parts.next()?.parse::<i64>().ok()?;
    let token = parts.next()?.to_string();
    Some((action, request_id, token))
}

fn new_childbot_memo(child_bot_id: i64) -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>()
        .to_ascii_uppercase();
    format!("CB{child_bot_id}{suffix}")
}

fn is_command(text: &str, command: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    first == command || first.starts_with(&format!("{command}@"))
}

fn is_childbot_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}
