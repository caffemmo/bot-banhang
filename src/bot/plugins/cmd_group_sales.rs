use std::sync::Arc;

use anyhow::Result;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, InlineKeyboardButton, InlineKeyboardMarkup, Message};
use url::Url;

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, i18n};
use crate::domains::products::models::Product;
use crate::domains::products::repo as products_repo;

pub struct GroupSalesCommandPlugin;

const GROUP_SHOP_COMMAND: &str = "/gshop";
const POST_PRODUCT_COMMAND: &str = "/postproduct";

#[async_trait::async_trait]
impl AppPlugin for GroupSalesCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdGroupSales"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![
            BotCommand {
                command: "gshop".to_string(),
                description: "Open the shop bot from a group".to_string(),
            },
            BotCommand {
                command: "postproduct".to_string(),
                description: "Admin: post a product card to this chat".to_string(),
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
        if is_command(text, GROUP_SHOP_COMMAND) {
            send_group_shop_entry(&ctx, &msg).await?;
            return Ok(true);
        }

        if is_command(text, POST_PRODUCT_COMMAND) {
            handle_post_product_command(&ctx, &msg, text).await?;
            return Ok(true);
        }

        Ok(false)
    }
}

async fn send_group_shop_entry(ctx: &AppContext, msg: &Message) -> Result<()> {
    let url = bot_start_url(ctx).await?;
    ctx.bot
        .send_message(
            msg.chat.id,
            "🛒 Mua hàng trong bot riêng để bảo mật thông tin đơn hàng và thanh toán.",
        )
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "🛒 Mở shop trong bot",
            url,
        )]]))
        .await?;
    Ok(())
}

async fn handle_post_product_command(ctx: &AppContext, msg: &Message, text: &str) -> Result<()> {
    let Some(user) = msg.from.as_ref() else {
        return Ok(());
    };
    if !is_group_sales_admin(ctx, user.id.0 as i64) {
        let lang = i18n::user_lang(ctx, user.id.0 as i64, user.language_code.as_deref()).await;
        ctx.bot
            .send_message(
                msg.chat.id,
                i18n::t(ctx, &lang, "unauthorized", "Unauthorized."),
            )
            .await?;
        return Ok(());
    }

    let Some(product_id) = text
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<i64>().ok())
    else {
        ctx.bot
            .send_message(msg.chat.id, "Cách dùng: /postproduct <product_id>")
            .await?;
        return Ok(());
    };

    let Some(product) = products_repo::get_product(&ctx.pool, product_id).await? else {
        ctx.bot
            .send_message(msg.chat.id, format!("Không tìm thấy sản phẩm ID {product_id}."))
            .await?;
        return Ok(());
    };

    if product.is_active.unwrap_or(1) == 0 {
        ctx.bot
            .send_message(msg.chat.id, "Sản phẩm này đang tắt, không đăng lên nhóm.")
            .await?;
        return Ok(());
    }

    let url = bot_start_url(ctx).await?;
    ctx.bot
        .send_message(msg.chat.id, product_group_card(&product))
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "🚀 Xem & mua trong bot",
            url,
        )]]))
        .await?;
    Ok(())
}

async fn bot_start_url(ctx: &AppContext) -> Result<Url> {
    let me = ctx.bot.get_me().await?;
    let username = me.user.username.unwrap_or_default();
    Url::parse(&format!("https://t.me/{username}?start=shop")).map_err(Into::into)
}

fn product_group_card(product: &Product) -> String {
    let mut lines = vec![
        format!("🛒 {}", product.name.trim()),
        format!("💰 Giá: {}", format_vnd(product.price)),
    ];

    if let Some(category) = product
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("📂 Danh mục: {category}"));
    }

    if let Some(description) = product
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push(truncate_chars(description, 700));
    }

    lines.push(String::new());
    lines.push("Bấm nút bên dưới để mở bot và mua hàng riêng tư.".to_string());
    lines.join("\n")
}

fn is_group_sales_admin(ctx: &AppContext, user_id: i64) -> bool {
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

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    output.push('…');
    output
}

fn format_vnd(amount: i64) -> String {
    let raw = amount.abs().to_string();
    let mut grouped = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut value = grouped.chars().rev().collect::<String>();
    if amount < 0 {
        value.insert(0, '-');
    }
    format!("{value} VND")
}
