use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use rand::{Rng, distributions::Alphanumeric};
use serde_json::{Value, json};
use std::path::Path;
use teloxide::payloads::{EditMessageCaptionSetters, EditMessageTextSetters, SendPhotoSetters};
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{
    CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, Message,
    MessageEntity, MessageId, ParseMode,
};
use tokio::time::{Duration as TokioDuration, sleep};
use tracing::warn;
use url::Url;

use crate::app::AppContext;
use crate::core::qr::vietqr_link;
use crate::domains::client::repo as client_repo;
use crate::domains::crypto_pay::models::{
    CryptoPaymentMethod, CryptoPaymentRequest, CryptoPaymentStatus,
};
use crate::domains::crypto_pay::{
    bep20 as bep20_pay, binance as binance_pay, binance_worker, repo as crypto_repo,
};
use crate::domains::orders::admin_notify::notify_admins_order_paid;
use crate::domains::orders::fulfillment::PaymentSource;
use crate::domains::orders::models::{Order, OrderReservationMode, OrderStatus, OrderWithProduct};
use crate::domains::orders::{api as orders_api, repo as orders_repo};
use crate::domains::products::models::Product;
use crate::domains::products::repo;
use crate::domains::wallet::repo as wallet_repo;

use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use teloxide::types::BotCommand;

const COUNTDOWN_TICK_SECONDS: u64 = 2;
const PRODUCT_BUTTON_NAME_MAX_CHARS: usize = 32;

fn order_expires_at(created_at: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&Utc) + Duration::minutes(orders_api::RESERVE_TTL_MINUTES))
        .unwrap_or_else(|_| Utc::now() + Duration::minutes(orders_api::RESERVE_TTL_MINUTES))
}

const NO_RESERVE_WINDOW_HOURS: i64 = 24;
const NO_RESERVE_MIN_UNPAID: i64 = 3;
const NO_RESERVE_MIN_UNPAID_RATIO: f64 = 0.70;

fn no_reserve_decision(summary: orders_repo::OrderRiskSummary) -> Option<String> {
    if summary.total_orders <= 0 || summary.unpaid_orders < NO_RESERVE_MIN_UNPAID {
        return None;
    }

    let ratio = summary.unpaid_orders as f64 / summary.total_orders as f64;
    if ratio < NO_RESERVE_MIN_UNPAID_RATIO {
        return None;
    }

    Some(format!(
        "recent unpaid orders {}/{} ({:.0}%)",
        summary.unpaid_orders,
        summary.total_orders,
        ratio * 100.0
    ))
}

fn stock_backed_order_reservation_mode(
    _available_stock: i64,
    _requested_qty: i64,
    risk_reason: Option<String>,
) -> (OrderReservationMode, Option<String>) {
    (OrderReservationMode::NoReserve, risk_reason)
}

async fn no_reserve_reason_for_user(ctx: &AppContext, user_id: i64) -> Result<Option<String>> {
    let window_started_at = (Utc::now() - Duration::hours(NO_RESERVE_WINDOW_HOURS)).to_rfc3339();
    let summary = orders_repo::order_risk_summary(&ctx.pool, user_id, &window_started_at).await?;
    Ok(no_reserve_decision(summary))
}

async fn notify_admins_for_no_reserve_order(
    ctx: &AppContext,
    order: &Order,
    product: &Product,
    reason: &str,
) {
    let text = format!(
        "Canh bao no-reserve order\nUser: {}\nChat: {}\nOrder memo: {}\nProduct: {}\nAmount: {}\nReason: {}\n\nUser van duoc dat don, nhung don nay khong giu stock. Ai thanh toan truoc se nhan hang neu con stock.",
        order.user_id,
        order.chat_id,
        order.bank_memo,
        product.name,
        format_vnd(order.amount),
        reason
    );

    for admin_id in ctx.telegram_icon_admin_ids() {
        if let Err(err) = ctx.bot.send_message(ChatId(admin_id), text.clone()).await {
            warn!("failed to notify admin {admin_id} about no-reserve order: {err}");
        }
    }
}

fn render_qr_caption(base_caption: &str, countdown_line: &str) -> String {
    format!("{base_caption}\n\n{countdown_line}")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn copyable_code(value: &str) -> String {
    format!("<code>{}</code>", html_escape(value))
}

fn keep_qr_keyboard_on_caption_edit<T>(request: T, keyboard: &InlineKeyboardMarkup) -> T
where
    T: EditMessageCaptionSetters,
{
    request.reply_markup(keyboard.clone())
}

fn product_image_path_candidates(image_url: &str) -> Vec<String> {
    let relative = image_url.trim().trim_start_matches('/');
    if let Some(filename) = relative.strip_prefix("uploads/") {
        return vec![
            format!("storage/uploads/{filename}"),
            format!("public/uploads/{filename}"),
        ];
    }
    vec![format!("public/{relative}")]
}

fn product_image_file_path(image_url: &str) -> Option<String> {
    product_image_path_candidates(image_url)
        .into_iter()
        .find(|path| Path::new(path).is_file())
}

async fn send_product_prompt(
    ctx: &Arc<AppContext>,
    chat_id: ChatId,
    image_url: Option<&str>,
    text: String,
    keyboard: InlineKeyboardMarkup,
) -> Result<()> {
    let rich = i18n::rich_text_for_key(ctx, "", text);
    if let Some(image_url) = image_url {
        if let Some(path) = product_image_file_path(image_url) {
            let mut request = ctx
                .bot
                .send_photo(chat_id, InputFile::file(path))
                .caption(rich.text.clone())
                .reply_markup(keyboard.clone());
            if !rich.entities.is_empty() {
                request = request.caption_entities(rich.entities.clone());
            }
            match request.await {
                Ok(_) => return Ok(()),
                Err(err) => warn!("Failed to send product image {image_url}: {err}"),
            }
        } else {
            warn!("Product image file not found for URL {image_url}");
        }
    }

    let mut request = ctx
        .bot
        .send_message(chat_id, rich.text)
        .reply_markup(keyboard);
    if !rich.entities.is_empty() {
        request = request.entities(rich.entities);
    }
    request.await?;
    Ok(())
}

fn mmss(remaining_secs: i64) -> String {
    let clamped = remaining_secs.max(0);
    let mins = clamped / 60;
    let secs = clamped % 60;
    format!("{mins:02}:{secs:02}")
}

async fn edit_caption_soft(
    ctx: &Arc<AppContext>,
    chat_id: ChatId,
    message_id: MessageId,
    caption: String,
    keyboard: &InlineKeyboardMarkup,
) -> Result<()> {
    for attempt in 0..4_u64 {
        let request = ctx
            .bot
            .edit_message_caption(chat_id, message_id)
            .caption(caption.clone())
            .parse_mode(ParseMode::Html);
        let result = keep_qr_keyboard_on_caption_edit(request, keyboard).await;
        match result {
            Ok(_) => return Ok(()),
            Err(err) => {
                let err_msg = err.to_string().to_lowercase();
                if err_msg.contains("message is not modified") {
                    return Ok(());
                }

                let is_retryable = err_msg.contains("too many requests")
                    || err_msg.contains("retry after")
                    || err_msg.contains("timeout")
                    || err_msg.contains("network")
                    || err_msg.contains("temporar");
                if is_retryable && attempt < 3 {
                    sleep(TokioDuration::from_millis(300 * (attempt + 1))).await;
                    continue;
                }

                return Err(err.into());
            }
        }
    }
    Ok(())
}

fn spawn_order_qr_countdown(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    message_id: MessageId,
    order_id: String,
    base_caption: String,
    expires_at: DateTime<Utc>,
    keyboard: InlineKeyboardMarkup,
    lang: String,
) {
    tokio::spawn(async move {
        loop {
            let Some(order) = orders_repo::get_order(&ctx.pool, &order_id)
                .await
                .ok()
                .flatten()
            else {
                break;
            };
            if !matches!(order.status, OrderStatus::Pending) {
                break;
            }

            let now = Utc::now();
            if now >= expires_at {
                let _ = edit_caption_soft(
                    &ctx,
                    chat_id,
                    message_id,
                    render_qr_caption(
                        &base_caption,
                        &tl(&ctx, &lang, "qr_expired", "⛔ QR has expired."),
                    ),
                    &keyboard,
                )
                .await;
                break;
            }

            let remaining = (expires_at - now).num_seconds();
            let line = trl(
                &ctx,
                &lang,
                "qr_countdown",
                "⏰ QR valid for: {time}",
                &[("time", mmss(remaining))],
            );
            if let Err(err) = edit_caption_soft(
                &ctx,
                chat_id,
                message_id,
                render_qr_caption(&base_caption, &line),
                &keyboard,
            )
            .await
            {
                warn!("failed to update order QR countdown {order_id}: {err}");
            }
            sleep(TokioDuration::from_secs(COUNTDOWN_TICK_SECONDS)).await;
        }
    });
}

async fn shop_handle_callback(
    ctx: Arc<AppContext>,
    q: CallbackQuery,
    dialogue: BotDialogue,
) -> Result<()> {
    let Some(data) = q.data.clone() else {
        return Ok(());
    };
    let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;

    let _ = ctx.bot.answer_callback_query(q.id.clone()).await;

    if data == "start:shop" {
        if let Some(msg) = &q.message {
            dialogue.update(State::Idle).await?;
            send_products(
                ctx.clone(),
                ctx.bot.clone(),
                msg.chat().id,
                0,
                Some(msg.id()),
                &lang,
            )
            .await?;
        }
    } else if let Some(page) = data.strip_prefix("shopnew:") {
        let page: i64 = page.parse().unwrap_or(0);
        if let Some(msg) = &q.message {
            dialogue.update(State::Idle).await?;
            send_products(
                ctx.clone(),
                ctx.bot.clone(),
                msg.chat().id,
                page,
                None,
                &lang,
            )
            .await?;
        }
    } else if data == "shop_api" || data == "shop_api_new" {
        if let Some(msg) = &q.message {
            dialogue.update(State::Idle).await?;
            show_api_integration_page(
                ctx.clone(),
                msg.chat().id,
                msg.id(),
                q.from.id.0 as i64,
                data == "shop_api_new",
                &lang,
            )
            .await?;
        }
    } else if let Some(category) = data.strip_prefix("shop_cat:") {
        if let Some(msg) = &q.message {
            dialogue.update(State::Idle).await?;
            send_products_for_category(
                ctx.clone(),
                msg.chat().id,
                msg.id(),
                category.trim(),
                &lang,
            )
            .await?;
        }
    } else if data.starts_with("shop:") {
        let page: i64 = data["shop:".len()..].parse().unwrap_or(0);
        if let Some(msg) = &q.message {
            send_products(
                ctx.clone(),
                ctx.bot.clone(),
                msg.chat().id,
                page,
                Some(msg.id()),
                &lang,
            )
            .await?;
        }
    } else if data.starts_with("buy:") {
        let product_id: i64 = data["buy:".len()..].parse().unwrap_or_default();
        if let Some(msg) = &q.message {
            let product = repo::get_product(&ctx.pool, product_id).await?;
            if let Some(product) = product {
                if !product_is_available_for_purchase(&product) {
                    ctx.bot
                        .send_message(
                            msg.chat().id,
                            tl(
                                &ctx,
                                &lang,
                                "product_unavailable",
                                "This product is no longer available. Please choose another product.",
                            ),
                        )
                        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                            i18n::inline_button_callback(
                                &ctx,
                                &lang,
                                "open_shop_btn",
                                "🛒 Open shop",
                                "start:shop",
                            ),
                        ]]))
                        .await?;
                    return Ok(());
                }
                let sold_count = if product.show_sold_count.unwrap_or(0) != 0 {
                    repo::count_product_paid_quantity_sold(&ctx.pool, product.id)
                        .await
                        .unwrap_or(0)
                } else {
                    0
                };
                let desc_text = product_description_prompt_line(&ctx, &lang, &product, sold_count);

                let delivery_type = orders_api::product_delivery_type(&product);
                if delivery_type == "uploaded_file" {
                    let stock = repo::count_product_items(&ctx.pool, product_id)
                        .await
                        .unwrap_or(0);
                    if !uploaded_file_has_sellable_stock(&product, stock) {
                        ctx.bot
                            .send_message(
                                msg.chat().id,
                                tl(
                                    &ctx,
                                    &lang,
                                    "uploaded_file_out_of_stock",
                                    "File stock is currently out. Please choose another product.",
                                ),
                            )
                            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                            .await?;
                        return Ok(());
                    }
                    dialogue.update(State::ChoosingQty { product_id }).await?;
                    let text = uploaded_file_quantity_prompt(
                        &product.name,
                        product.price,
                        stock,
                        Some(desc_text.as_str()),
                        &ctx,
                        &lang,
                    );
                    send_product_prompt(
                        &ctx,
                        msg.chat().id,
                        product.image_url.as_deref(),
                        text,
                        quantity_keyboard(&ctx, &lang, false),
                    )
                    .await?;
                    return Ok(());
                }

                if delivery_type == "manual_input" {
                    let plans = repo::list_product_plans(&ctx.pool, product_id).await?;
                    if !plans.is_empty() {
                        dialogue.update(State::SelectingPlan { product_id }).await?;
                        let text = format!(
                            "{}",
                            trl(
                                &ctx,
                                &lang,
                                "manual_product_plan_prompt",
                                "✅ You selected {product} — {price}\n{description}ℹ️ This product requires activation information.\n\n📅 Choose a plan/month below:",
                                &[
                                    ("product", product.name.clone()),
                                    ("price", format_vnd(product.price)),
                                    ("description", desc_text.clone()),
                                ],
                            )
                        );

                        send_product_prompt(
                            &ctx,
                            msg.chat().id,
                            product.image_url.as_deref(),
                            text,
                            plan_keyboard(&ctx, &lang, &plans),
                        )
                        .await?;
                        return Ok(());
                    }
                }

                let stock = repo::count_product_items(&ctx.pool, product_id)
                    .await
                    .unwrap_or(0);
                dialogue.update(State::ChoosingQty { product_id }).await?;
                let text = format!(
                    "{}",
                    trl(
                        &ctx,
                        &lang,
                        "product_qty_prompt",
                        "✅ You selected {product} — {price}\n📦 Stock left: {stock}\n{description}{requires_input}\n\n⌨️ Enter quantity to buy:",
                        &[
                            ("product", product.name.clone()),
                            ("price", format_vnd(product.price)),
                            ("stock", stock.to_string()),
                            ("description", description_for_quantity_prompt(&desc_text)),
                            (
                                "requires_input",
                                if delivery_type == "manual_input" {
                                    tl(
                                        &ctx,
                                        &lang,
                                        "product_requires_input_note",
                                        "ℹ️ This product requires activation information, which will be requested in the next step.",
                                    )
                                } else {
                                    "".to_string()
                                },
                            ),
                        ],
                    )
                );

                send_product_prompt(
                    &ctx,
                    msg.chat().id,
                    product.image_url.as_deref(),
                    text,
                    quantity_keyboard(&ctx, &lang, false),
                )
                .await?;
            }
        }
    } else if data.starts_with("qty:") {
        let qty: i64 = data["qty:".len()..].parse().unwrap_or(1);
        let state = dialogue.get().await?;
        if let Some(State::ChoosingQty { product_id }) = state {
            if let Some(msg) = &q.message {
                handle_qty_chosen(
                    ctx.clone(),
                    msg.chat().id,
                    q.from.id.0 as i64,
                    dialogue,
                    product_id,
                    qty,
                    &lang,
                )
                .await?;
            }
        } else if let Some(msg) = &q.message {
            ctx.bot
                .send_message(
                    msg.chat().id,
                    tl(
                        &ctx,
                        &lang,
                        "session_expired",
                        "Session expired. Please type /shop to buy again.",
                    ),
                )
                .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                .await?;
        }
    } else if data.starts_with("plan:") {
        let plan_id: i64 = data["plan:".len()..].parse().unwrap_or(0);
        let state = dialogue.get().await?;
        if let Some(State::SelectingPlan { product_id }) = state {
            if let Some(msg) = &q.message {
                let plan = repo::get_product_plan(&ctx.pool, plan_id).await?;
                if let Some(plan) = plan {
                    if plan.product_id != product_id {
                        ctx.bot
                            .send_message(
                                msg.chat().id,
                                tl(
                                    &ctx,
                                    &lang,
                                    "plan_invalid_for_product",
                                    "This plan is invalid for this product.",
                                ),
                            )
                            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                            .await?;
                    } else {
                        dialogue
                            .update(State::CollectingInfo {
                                product_id,
                                qty: plan.months,
                                plan_id: Some(plan.id),
                            })
                            .await?;
                        let prompt = repo::get_product(&ctx.pool, product_id)
                            .await?
                            .and_then(|p| p.input_prompt)
                            .unwrap_or_else(|| {
                                tl(
                                    &ctx,
                                    &lang,
                                    "default_input_prompt",
                                    "Enter email or activation information to complete:",
                                )
                            });
                        ctx.bot
                            .send_message(
                                msg.chat().id,
                                trl(
                                    &ctx,
                                    &lang,
                                    "plan_chosen",
                                    "Selected plan: {label} ({price})\n📝 {prompt}",
                                    &[
                                        ("label", plan.label.clone()),
                                        ("price", format_vnd(plan.price)),
                                        ("prompt", prompt.clone()),
                                    ],
                                ),
                            )
                            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                            .await?;
                    }
                } else {
                    ctx.bot
                        .send_message(
                            msg.chat().id,
                            tl(&ctx, &lang, "plan_not_found", "Plan does not exist."),
                        )
                        .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                        .await?;
                }
            }
        } else if let Some(msg) = &q.message {
            ctx.bot
                .send_message(
                    msg.chat().id,
                    tl(
                        &ctx,
                        &lang,
                        "session_expired",
                        "Session expired. Please type /shop to try again.",
                    ),
                )
                .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                .await?;
        }
    } else if let Some(order_id) = data.strip_prefix("cancel:") {
        if let Some(msg) = &q.message {
            // NOTE: CallbackQuery::from is the user who clicked the button.
            // msg.from() is the sender of the message (usually the bot), so using it
            // will incorrectly fail the ownership check.
            let user_id = q.from.id.0 as i64;

            let Some(order) = repo::get_order(&ctx.pool, order_id).await? else {
                ctx.bot
                    .send_message(
                        msg.chat().id,
                        tl(&ctx, &lang, "order_not_found", "Order not found."),
                    )
                    .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            };
            if order.user_id != user_id {
                ctx.bot
                    .send_message(
                        msg.chat().id,
                        tl(
                            &ctx,
                            &lang,
                            "cancel_not_owner",
                            "You cannot cancel this order.",
                        ),
                    )
                    .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            }
            if matches!(order.status, OrderStatus::Paid) {
                ctx.bot
                    .send_message(
                        msg.chat().id,
                        tl(
                            &ctx,
                            &lang,
                            "cancel_already_paid",
                            "This order is already paid and cannot be cancelled.",
                        ),
                    )
                    .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            }
            let mut tx = ctx.pool.begin().await?;
            if let Some(ids_str) = &order.reserved_item_ids {
                let ids = parse_reserved_ids(ids_str);
                if !ids.is_empty() {
                    repo::return_product_items(&mut tx, order.product_id, &ids).await?;
                }
            }
            repo::update_order_status_with_data(
                &mut tx,
                &order.id,
                OrderStatus::Cancel,
                None,
                None,
            )
            .await?;
            tx.commit().await?;
            ctx.bot
                .send_message(
                    msg.chat().id,
                    tl(&ctx, &lang, "order_cancelled", "Order has been cancelled."),
                )
                .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                .await?;
            send_products(ctx.clone(), ctx.bot.clone(), msg.chat().id, 0, None, &lang).await?;
        }
    } else if let Some(order_id) = data.strip_prefix("paywallet:") {
        if let Some(msg) = &q.message {
            let user_id = q.from.id.0 as i64;
            handle_pay_with_wallet(ctx.clone(), msg.chat().id, msg.id(), user_id, order_id).await?;
        }
    } else if let Some(rest) = data.strip_prefix("cryptopay:") {
        if let Some(msg) = &q.message {
            let user_id = q.from.id.0 as i64;
            handle_crypto_callback(ctx.clone(), msg.chat().id, user_id, rest).await?;
        }
    } else if let Some(msg) = &q.message {
        ctx.bot
            .send_message(
                msg.chat().id,
                tl(
                    &ctx,
                    &lang,
                    "action_invalid",
                    "Invalid action. Please try again.",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
    }

    Ok(())
}

async fn handle_crypto_callback(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    data: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    if let Some(order_id) = data.strip_prefix("bep20:") {
        handle_bep20_checkout(ctx, chat_id, user_id, order_id).await?;
    } else if let Some(order_id) = data.strip_prefix("binance:") {
        handle_binance_checkout(ctx, chat_id, user_id, order_id).await?;
    } else if let Some(payment_id) = data.strip_prefix("copy_address:") {
        handle_crypto_copy(ctx, chat_id, user_id, payment_id, CryptoCopyField::Address).await?;
    } else if let Some(payment_id) = data.strip_prefix("copy_amount:") {
        handle_crypto_copy(ctx, chat_id, user_id, payment_id, CryptoCopyField::Amount).await?;
    } else if let Some(payment_id) = data.strip_prefix("check:") {
        handle_crypto_check(ctx, chat_id, user_id, payment_id).await?;
    } else if let Some(payment_id) = data.strip_prefix("cancel:") {
        handle_crypto_cancel(ctx, chat_id, user_id, payment_id).await?;
    } else {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "crypto_action_invalid",
                    "Invalid payment action.",
                ),
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
    }
    Ok(())
}

async fn handle_binance_checkout(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    match binance_pay::create_or_reuse_binance_payment(ctx.clone(), order_id, user_id, chat_id.0)
        .await
    {
        Ok(payment) => {
            let amount = bep20_pay::format_bep20_amount(payment.amount_usdt_expected);
            let pay_id = ctx.binance_pay_receiver_pay_id().unwrap_or_default();
            let receiver_name = ctx.binance_pay_receiver_name().unwrap_or_default();
            let text = trl(
                &ctx,
                &lang,
                "binance_payment_note_instructions",
                "🟡 THANH TOÁN USDT QUA BINANCE PAY\n\n━━━━━━━━━━━━━━━━━━━\n💵 Số USDT cần chuyển: {amount} USDT\n💰 Đơn hàng: {order_id}\n━━━━━━━━━━━━━━━━━━━\n\n📲 Thông tin Binance Pay:\n• Pay ID: {pay_id}\n• Binance ID: {receiver_name}\n\n📝 Nội dung ghi chú: {memo}\n\n⚠️ Ghi chính xác mã nội dung để hệ thống nhận biết.\n⏱️ Hệ thống tự động xác nhận sau 1-2 phút.\n⏳ Hết hạn: {expires_at}",
                &[
                    ("amount", copyable_code(&amount)),
                    (
                        "order_id",
                        copyable_code(payment.order_id.as_deref().unwrap_or("-")),
                    ),
                    ("pay_id", copyable_code(&pay_id)),
                    ("receiver_name", html_escape(&receiver_name)),
                    ("memo", copyable_code(&payment.memo)),
                    ("expires_at", payment.expires_at.clone()),
                ],
            );
            ctx.bot
                .send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(build_crypto_payment_keyboard(&ctx, &lang, payment.id))
                .await?;
        }
        Err(err) => {
            ctx.bot
                .send_message(
                    chat_id,
                    trl(
                        &ctx,
                        &lang,
                        "binance_payment_error",
                        "Could not create Binance Pay checkout: {error}",
                        &[("error", err.to_string())],
                    ),
                )
                .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
                .await?;
        }
    }
    Ok(())
}

async fn handle_bep20_checkout(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    match bep20_pay::create_or_reuse_bep20_payment(ctx.clone(), order_id, user_id, chat_id.0).await
    {
        Ok(payment) => {
            let address = payment.address.as_deref().unwrap_or("");
            let amount = bep20_pay::format_bep20_amount(payment.amount_usdt_expected);
            let text = trl(
                &ctx,
                &lang,
                "bep20_payment_instructions",
                "🟢 USDT BEP20 payment\n\n• Network: BNB Smart Chain (BEP20)\n• Amount to send: {amount} USDT\n• Receiving address: {address}\n• Order: {order_id}\n• Expires: {expires_at}\n\n⚠️ Send exactly {amount} USDT on BNB Smart Chain (BEP20). Do not round, underpay, or overpay. Wrong amount or wrong network requires manual review and may delay delivery.",
                &[
                    ("amount", copyable_code(&amount)),
                    ("address", copyable_code(address)),
                    (
                        "order_id",
                        copyable_code(payment.order_id.as_deref().unwrap_or("-")),
                    ),
                    ("expires_at", payment.expires_at.clone()),
                ],
            );
            ctx.bot
                .send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(build_crypto_payment_keyboard(&ctx, &lang, payment.id))
                .await?;
        }
        Err(err) => {
            ctx.bot
                .send_message(
                    chat_id,
                    trl(
                        &ctx,
                        &lang,
                        "bep20_payment_error",
                        "Could not create USDT BEP20 payment: {error}",
                        &[("error", err.to_string())],
                    ),
                )
                .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
                .await?;
        }
    }
    Ok(())
}

fn build_crypto_payment_keyboard(
    ctx: &AppContext,
    lang: &str,
    payment_id: i64,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            i18n::inline_button_callback(
                ctx,
                lang,
                "copy_address_btn",
                "Copy address",
                format!("cryptopay:copy_address:{payment_id}"),
            ),
            i18n::inline_button_callback(
                ctx,
                lang,
                "copy_amount_btn",
                "Copy amount",
                format!("cryptopay:copy_amount:{payment_id}"),
            ),
        ],
        vec![
            i18n::inline_button_callback(
                ctx,
                lang,
                "check_crypto_btn",
                "Check payment",
                format!("cryptopay:check:{payment_id}"),
            ),
            i18n::inline_button_callback(
                ctx,
                lang,
                "cancel_crypto_btn",
                "Cancel USDT",
                format!("cryptopay:cancel:{payment_id}"),
            ),
        ],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "shop_back_btn",
            "⬅️ Quay lại",
            "start:shop",
        )],
    ])
}

fn crypto_action_result_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    shop_action_result_keyboard(ctx, lang)
}

fn shop_action_result_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        i18n::inline_button_callback(ctx, lang, "shop_back_btn", "⬅️ Quay lại", "start:shop"),
        i18n::inline_button_callback(ctx, lang, "start_btn_wallet", "💳 Wallet", "start:wallet"),
    ]])
}

enum CryptoCopyField {
    Address,
    Amount,
}

async fn handle_crypto_copy(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    payment_id: &str,
    field: CryptoCopyField,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    let Some(payment) = crypto_payment_for_user(&ctx, payment_id, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "crypto_payment_not_found",
                    "Payment request not found.",
                ),
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    };
    let value = match field {
        CryptoCopyField::Address => payment.address.unwrap_or_default(),
        CryptoCopyField::Amount => bep20_pay::format_bep20_amount(payment.amount_usdt_expected),
    };
    ctx.bot
        .send_message(chat_id, copyable_code(&value))
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn handle_crypto_check(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    payment_id: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    let Some(payment) = crypto_payment_for_user(&ctx, payment_id, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "crypto_payment_not_found",
                    "Payment request not found.",
                ),
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    };
    if payment.method == CryptoPaymentMethod::BinancePay
        && matches!(
            payment.status,
            CryptoPaymentStatus::Pending | CryptoPaymentStatus::Confirming
        )
    {
        match sync_binance_payment(ctx.clone(), &payment).await {
            Ok(Some(message)) => {
                ctx.bot
                    .send_message(chat_id, message)
                    .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                ctx.bot
                    .send_message(
                        chat_id,
                        trl(
                            &ctx,
                            &lang,
                            "binance_query_error",
                            "Could not query Binance Pay status: {error}",
                            &[("error", err.to_string())],
                        ),
                    )
                    .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            }
        }
    }
    let message = match payment.status {
        CryptoPaymentStatus::Pending => tl(
            &ctx,
            &lang,
            "crypto_status_pending",
            "Payment is still pending. I will update the order automatically after the transfer is detected.",
        ),
        CryptoPaymentStatus::Confirming => trl(
            &ctx,
            &lang,
            "crypto_status_confirming",
            "Transfer detected and waiting for confirmations: {confirmations}.",
            &[("confirmations", payment.confirmations.to_string())],
        ),
        CryptoPaymentStatus::Completed => tl(
            &ctx,
            &lang,
            "crypto_status_completed",
            "Payment completed. Your order is being delivered.",
        ),
        CryptoPaymentStatus::Expired => tl(
            &ctx,
            &lang,
            "crypto_status_expired",
            "This payment request has expired. Create a new USDT payment if you still want to pay.",
        ),
        CryptoPaymentStatus::Failed => payment.failure_reason.unwrap_or_else(|| {
            tl(
                &ctx,
                &lang,
                "crypto_status_failed",
                "Payment failed. Please contact support.",
            )
        }),
        CryptoPaymentStatus::ManualReview => tl(
            &ctx,
            &lang,
            "crypto_status_manual_review",
            "Payment is under manual review. Please wait for admin support.",
        ),
    };
    ctx.bot
        .send_message(chat_id, message)
        .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
        .await?;
    Ok(())
}

async fn sync_binance_payment(
    ctx: Arc<AppContext>,
    payment: &crate::domains::crypto_pay::models::CryptoPaymentRequest,
) -> Result<Option<String>> {
    if let Err(err) = binance_worker::run_binance_pay_tick(ctx.clone()).await {
        warn!(
            "on-demand Binance Pay note scan failed for {}: {err}",
            payment.id
        );
    }
    let refreshed = crypto_repo::find_crypto_payment_by_id(&ctx.pool, payment.id)
        .await?
        .unwrap_or_else(|| payment.clone());
    if refreshed.status == CryptoPaymentStatus::Completed {
        Ok(Some(
            "Binance Pay payment completed. Your order is being delivered.".to_string(),
        ))
    } else {
        Ok(None)
    }
}

async fn handle_crypto_cancel(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    payment_id: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    let Some(payment) = crypto_payment_for_user(&ctx, payment_id, user_id, chat_id.0).await? else {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "crypto_payment_not_found",
                    "Payment request not found.",
                ),
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    };
    if matches!(
        payment.status,
        CryptoPaymentStatus::Confirming
            | CryptoPaymentStatus::Completed
            | CryptoPaymentStatus::Failed
            | CryptoPaymentStatus::ManualReview
    ) {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "crypto_cancel_not_pending",
                    "Only pending payment requests can be cancelled.",
                ),
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }
    if payment.order_id.is_some() {
        let cancelled = cancel_order_for_crypto_payment(&ctx, &payment).await?;
        ctx.bot
            .send_message(
                chat_id,
                if cancelled {
                    tl(
                        &ctx,
                        &lang,
                        "crypto_cancelled",
                        "USDT payment request has been cancelled.",
                    )
                } else {
                    tl(
                        &ctx,
                        &lang,
                        "crypto_cancel_not_pending",
                        "Only pending payment requests can be cancelled.",
                    )
                },
            )
            .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }
    let cancelled = crypto_repo::expire_crypto_payment(&ctx.pool, payment.id).await?;
    ctx.bot
        .send_message(
            chat_id,
            if cancelled {
                tl(
                    &ctx,
                    &lang,
                    "crypto_cancelled",
                    "USDT payment request has been cancelled.",
                )
            } else {
                tl(
                    &ctx,
                    &lang,
                    "crypto_cancel_not_pending",
                    "Only pending payment requests can be cancelled.",
                )
            },
        )
        .reply_markup(crypto_action_result_keyboard(&ctx, &lang))
        .await?;
    Ok(())
}

async fn cancel_order_for_crypto_payment(
    ctx: &AppContext,
    payment: &CryptoPaymentRequest,
) -> Result<bool> {
    if !matches!(
        payment.status,
        CryptoPaymentStatus::Pending | CryptoPaymentStatus::Expired
    ) {
        return Ok(false);
    }
    let Some(order_id) = payment.order_id.as_deref() else {
        return Ok(false);
    };
    let Some(order) = orders_repo::get_order(&ctx.pool, order_id).await? else {
        return Ok(false);
    };
    if order.user_id != payment.user_id || order.chat_id != payment.chat_id {
        return Ok(false);
    }
    if !matches!(order.status, OrderStatus::Pending) {
        return Ok(false);
    }

    let mut tx = ctx.pool.begin().await?;
    if matches!(payment.status, CryptoPaymentStatus::Pending) {
        let expired = crypto_repo::expire_crypto_payment_tx(&mut tx, payment.id).await?;
        if !expired {
            tx.rollback().await?;
            return Ok(false);
        }
    }
    if let Some(ids_str) = &order.reserved_item_ids {
        let ids = parse_reserved_ids(ids_str);
        if !ids.is_empty() {
            repo::return_product_items(&mut tx, order.product_id, &ids).await?;
        }
    }
    orders_repo::update_order_status_with_data(&mut tx, &order.id, OrderStatus::Cancel, None, None)
        .await?;
    tx.commit().await?;
    Ok(true)
}

async fn crypto_payment_for_user(
    ctx: &AppContext,
    payment_id: &str,
    user_id: i64,
    chat_id: i64,
) -> Result<Option<crate::domains::crypto_pay::models::CryptoPaymentRequest>> {
    let Ok(payment_id) = payment_id.parse::<i64>() else {
        return Ok(None);
    };
    let Some(payment) = crypto_repo::find_crypto_payment_by_id(&ctx.pool, payment_id).await? else {
        return Ok(None);
    };
    if payment.user_id == user_id && payment.chat_id == chat_id {
        Ok(Some(payment))
    } else {
        Ok(None)
    }
}

async fn handle_pay_with_wallet(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    msg_id: MessageId,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;

    let Some(owp) = orders_repo::get_order_with_product(&ctx.pool, order_id).await? else {
        ctx.bot
            .send_message(
                chat_id,
                tl(&ctx, &lang, "order_not_found", "Order not found."),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    };

    if owp.order.user_id != user_id {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "cancel_not_owner",
                    "You cannot pay this order.",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    if !matches!(owp.order.status, OrderStatus::Pending) {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "order_not_pending",
                    "⚠️ This order is no longer waiting for payment.",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    // Kiểm tra số dư ví
    let wallet = wallet_repo::get_or_create_wallet(&ctx.pool, user_id).await?;
    if wallet.balance < owp.order.amount {
        ctx.bot
            .send_message(
                chat_id,
                trl(
                    &ctx,
                    &lang,
                    "wallet_balance_not_enough",
                    "⚠️ Wallet balance is not enough.\nCurrent balance: {balance}\nRequired: {required}",
                    &[
                        ("balance", format_vnd(wallet.balance)),
                        ("required", format_vnd(owp.order.amount)),
                    ],
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    let (delivered_data, reserved_item_ids, balance_after, paid_at) =
        complete_wallet_payment_transaction(&ctx.pool, &owp, user_id, order_id).await?;

    // Cập nhật message
    let done_text = format!(
        "{}",
        trl(
            &ctx,
            &lang,
            "wallet_payment_success",
            "✅ Wallet payment successful!\nRemaining balance: {balance}",
            &[("balance", format_vnd(balance_after))],
        )
    );
    let continue_kb = InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            &ctx,
            &lang,
            "continue_shopping_btn",
            "🛒 Continue shopping",
            "shopnew:0",
        )],
        vec![i18n::inline_button_callback(
            &ctx,
            &lang,
            "start_btn_wallet",
            "💳 Wallet",
            "start:wallet",
        )],
    ]);
    let edit_result = ctx
        .bot
        .edit_message_caption(chat_id, msg_id)
        .caption(&done_text)
        .reply_markup(continue_kb.clone())
        .await;
    if edit_result.is_err() {
        let _ = ctx
            .bot
            .edit_message_text(chat_id, msg_id, &done_text)
            .reply_markup(continue_kb)
            .await;
    }

    // Gửi file sản phẩm
    let updated_owp = OrderWithProduct {
        order: {
            let mut o = owp.order.clone();
            o.status = OrderStatus::Paid;
            o.payment_tx_id = Some("wallet".to_string());
            o.paid_at = Some(paid_at.to_rfc3339());
            o.delivered_data = Some(delivered_data.clone());
            o.reserved_item_ids = reserved_item_ids;
            o
        },
        product: owp.product.clone(),
    };
    if let Err(e) = orders_api::send_product_file(&ctx, &updated_owp, &delivered_data).await {
        tracing::error!("send_product_file after wallet payment failed: {e}");
    }
    if let Err(e) = notify_admins_order_paid(
        &ctx,
        &updated_owp,
        "wallet",
        paid_at,
        &PaymentSource::Wallet,
    )
    .await
    {
        tracing::error!("send paid-order admin notification after wallet payment failed: {e}");
    }

    Ok(())
}

async fn complete_wallet_payment_transaction(
    pool: &crate::db::DbPool,
    owp: &OrderWithProduct,
    user_id: i64,
    order_id: &str,
) -> Result<(String, Option<String>, i64, chrono::DateTime<Utc>)> {
    let paid_at = Utc::now();
    let mut tx = pool.begin().await?;
    let mut reserved_item_ids = owp.order.reserved_item_ids.clone();

    let delivered_data = if let Some(data) = &owp.order.delivered_data {
        data.clone()
    } else if orders_api::product_delivery_type(&owp.product) == "uploaded_file" {
        return Err(anyhow!(
            "uploaded-file order is missing reserved delivery data; please recreate the order."
        ));
    } else {
        let reserved = repo::take_product_items(&mut tx, owp.order.product_id, owp.order.qty)
            .await
            .map_err(|e| anyhow!("Không đủ hàng trong kho: {e}"))?;
        let data = reserved
            .iter()
            .map(|i| i.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            return Err(anyhow!("Kho hàng trống"));
        }
        reserved_item_ids = Some(
            reserved
                .iter()
                .map(|item| item.id.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
        data
    };

    let balance_after = wallet_repo::debit_wallet(
        &mut tx,
        user_id,
        owp.order.amount,
        order_id,
        Some("wallet_purchase"),
    )
    .await?;
    orders_repo::mark_order_paid(
        &mut tx,
        order_id,
        "wallet",
        paid_at,
        Some(&delivered_data),
        reserved_item_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    Ok((delivered_data, reserved_item_ids, balance_after, paid_at))
}

async fn handle_qty_message(
    ctx: Arc<AppContext>,
    msg: Message,
    dialogue: BotDialogue,
    product_id: i64,
) -> Result<()> {
    let lang = if let Some(user) = msg.from() {
        i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
    } else {
        "en".to_string()
    };
    let qty_raw = msg.text().unwrap_or("").to_string();
    let qty: i64 = qty_raw.trim().parse().unwrap_or(0);
    if !(1..=999).contains(&qty) {
        ctx.bot
            .send_message(
                msg.chat.id,
                tl(
                    &ctx,
                    &lang,
                    "qty_invalid",
                    "Invalid quantity. Enter 1..999.",
                ),
            )
            .reply_markup(quantity_keyboard(&ctx, &lang, false))
            .await?;
        return Ok(());
    }

    let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    handle_qty_chosen(ctx, msg.chat.id, user_id, dialogue, product_id, qty, &lang).await?;
    Ok(())
}

async fn handle_qty_chosen(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    dialogue: BotDialogue,
    product_id: i64,
    qty: i64,
    lang: &str,
) -> Result<()> {
    let Some(product) = repo::get_product(&ctx.pool, product_id).await? else {
        ctx.bot
            .send_message(
                chat_id,
                tl(&ctx, lang, "product_not_found", "Product does not exist."),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, lang))
            .await?;
        dialogue.update(State::Idle).await?;
        return Ok(());
    };

    if orders_api::product_delivery_type(&product) == "manual_input" {
        dialogue
            .update(State::CollectingInfo {
                product_id,
                qty,
                plan_id: None,
            })
            .await?;
        let prompt = product.input_prompt.clone().unwrap_or_else(|| {
            tl(
                &ctx,
                lang,
                "default_input_prompt",
                "Enter email or activation information to complete:",
            )
        });
        ctx.bot
            .send_message(
                chat_id,
                trl(
                    &ctx,
                    lang,
                    "info_prompt",
                    "📝 {prompt}\n\nPlease enter it accurately so the order can be processed.",
                    &[("prompt", prompt.clone())],
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, lang))
            .await?;
        return Ok(());
    }

    process_order(ctx, chat_id, user_id, dialogue, product_id, qty, None, None).await
}

async fn handle_info_message(
    ctx: Arc<AppContext>,
    msg: Message,
    dialogue: BotDialogue,
    product_id: i64,
    qty: i64,
    plan_id: Option<i64>,
) -> Result<()> {
    let user_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    let lang = if let Some(user) = msg.from() {
        i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
    } else {
        "en".to_string()
    };
    let input = msg.text().unwrap_or("").trim().to_string();
    if input.is_empty() {
        ctx.bot
            .send_message(
                msg.chat.id,
                tl(
                    &ctx,
                    &lang,
                    "info_empty",
                    "Information cannot be empty. Please enter it again (example: email).",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    if input.len() > 200 {
        ctx.bot
            .send_message(
                msg.chat.id,
                tl(
                    &ctx,
                    &lang,
                    "info_too_long",
                    "Information is too long (maximum 200 characters).",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    process_order(
        ctx,
        msg.chat.id,
        user_id,
        dialogue,
        product_id,
        qty,
        Some(input),
        plan_id,
    )
    .await
}

pub(crate) async fn send_products(
    ctx: Arc<AppContext>,
    bot: Bot,
    chat_id: ChatId,
    _page: i64,
    target_message: Option<MessageId>,
    lang: &str,
) -> Result<()> {
    let total = repo::count_products(&ctx.pool).await?;
    if total == 0 {
        if let Some(msg_id) = target_message {
            bot.edit_message_text(
                chat_id,
                msg_id,
                tl(&ctx, lang, "no_products", "There are no products yet."),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, lang))
            .await?;
        } else {
            bot.send_message(
                chat_id,
                tl(&ctx, lang, "no_products", "There are no products yet."),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, lang))
            .await?;
        }
        return Ok(());
    }

    let products = repo::list_products(&ctx.pool, total, 0).await?;
    let mut products_with_stock = Vec::new();
    for p in products {
        let stock = repo::count_product_items(&ctx.pool, p.id)
            .await
            .unwrap_or(0);
        products_with_stock.push((p, stock));
    }

    let categories = category_buttons_from_products(&products_with_stock);
    let uncategorized_products = uncategorized_products_from_products(&products_with_stock);
    let raw_keyboard =
        build_shop_home_keyboard_json(&ctx, lang, &categories, &uncategorized_products);
    let wallet_balance = wallet_repo::get_or_create_wallet(&ctx.pool, chat_id.0)
        .await
        .map(|wallet| wallet.balance)
        .unwrap_or(0);
    let text = format_product_list_text(&ctx, lang, &products_with_stock, 0, wallet_balance);

    if let Some(msg_id) = target_message {
        send_raw_product_list_message(&ctx, chat_id, Some(msg_id), &text, raw_keyboard).await?;
    } else {
        send_raw_product_list_message(&ctx, chat_id, None, &text, raw_keyboard).await?;
    }

    Ok(())
}

async fn send_products_for_category(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    message_id: MessageId,
    category: &str,
    lang: &str,
) -> Result<()> {
    let products = repo::list_products_by_category(&ctx.pool, category).await?;
    let mut products_with_stock = Vec::new();
    for p in products {
        let stock = repo::count_product_items(&ctx.pool, p.id)
            .await
            .unwrap_or(0);
        products_with_stock.push((p, stock));
    }

    let keyboard = build_category_product_keyboard_json(&ctx, lang, &products_with_stock);
    let text = if products_with_stock.is_empty() {
        tl(&ctx, lang, "no_products", "There are no products yet.")
    } else {
        format_product_list_text(&ctx, lang, &products_with_stock, 0, 0)
    };
    send_raw_product_list_message(&ctx, chat_id, Some(message_id), &text, keyboard).await
}

async fn send_raw_product_list_message(
    ctx: &AppContext,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    text: &str,
    reply_markup: Value,
) -> Result<()> {
    let mut payload = shop_product_list_payload(ctx, chat_id, text, reply_markup.clone())?;
    if let Some(message_id) = message_id {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("message_id".to_string(), json!(message_id.0));
        }
        match i18n::send_raw_telegram_method(ctx, "editMessageText", payload).await {
            Ok(()) => Ok(()),
            Err(err) if should_fallback_to_new_product_list_message(&err) => {
                let payload = shop_product_list_payload(ctx, chat_id, text, reply_markup)?;
                i18n::send_raw_telegram_method(ctx, "sendMessage", payload).await
            }
            Err(err) => Err(err),
        }
    } else {
        i18n::send_raw_telegram_method(ctx, "sendMessage", payload).await
    }
}

fn should_fallback_to_new_product_list_message(err: &anyhow::Error) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("there is no text in the message to edit")
        || message.contains("message to edit not found")
        || message.contains("message can't be edited")
}

async fn show_api_integration_page(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    message_id: MessageId,
    user_id: i64,
    rotate_token: bool,
    lang: &str,
) -> Result<()> {
    let token = if rotate_token {
        client_repo::create_or_replace_api_key(&ctx.pool, user_id).await?
    } else {
        client_repo::get_or_create_api_key(&ctx.pool, user_id).await?
    };
    let text = format_api_integration_text(&ctx, user_id, &token);
    ctx.bot
        .edit_message_text(chat_id, message_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(api_integration_keyboard(&ctx, lang))
        .await?;
    Ok(())
}

pub(crate) async fn send_api_integration_page(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    rotate_token: bool,
    lang: &str,
) -> Result<()> {
    let token = if rotate_token {
        client_repo::create_or_replace_api_key(&ctx.pool, user_id).await?
    } else {
        client_repo::get_or_create_api_key(&ctx.pool, user_id).await?
    };
    let text = format_api_integration_text(&ctx, user_id, &token);
    ctx.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(api_integration_keyboard(&ctx, lang))
        .await?;
    Ok(())
}

fn format_api_integration_text(ctx: &AppContext, chat_id: i64, token: &str) -> String {
    let api_key = format!("{chat_id}:{token}");
    let base_url = api_base_url(ctx);
    let products_url = format!("{base_url}/api/client/products");
    let wallet_url = format!("{base_url}/api/client/wallet");
    let orders_url = format!("{base_url}/api/client/orders");
    let buy_example = "{\n  \"product_id\": 1,\n  \"qty\": 1,\n  \"plan_id\": null,\n  \"customer_input\": \"email@example.com\"\n}";

    format!(
        "🔌 <b>Tích hợp API</b>\n\n\
API key:\n{api_key}\n\n\
Base URL:\n{base_url}\n\n\
Endpoints:\nGET products: {products}\nGET wallet: {wallet}\nPOST /api/client/orders: {orders}\n\n\
Header:\n{header}\n\n\
Ví dụ body mua hàng:\n<pre>{buy_example}</pre>\n\n\
Bấm “Tạo token mới” nếu muốn đổi key. Token cũ sẽ mất hiệu lực ngay.",
        api_key = copyable_code(&api_key),
        base_url = copyable_code(&base_url),
        products = copyable_code(&products_url),
        wallet = copyable_code(&wallet_url),
        orders = copyable_code(&orders_url),
        header = copyable_code(&format!("Authorization: Bearer {api_key}")),
        buy_example = html_escape(buy_example),
    )
}

fn api_base_url(ctx: &AppContext) -> String {
    ctx.base_url()
        .unwrap_or_else(|| format!("http://localhost:{}", ctx.config.port))
        .trim_end_matches('/')
        .to_string()
}

fn api_integration_keyboard(ctx: &AppContext, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "shop_api_new_token_btn",
            "🔄 Tạo token mới",
            "shop_api_new",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "shop_back_btn",
            "⬅️ Quay lại",
            "start:shop",
        )],
    ])
}

fn api_integration_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "inline_keyboard": [
            [
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "shop_api_new_token_btn",
                    "🔄 Tạo token mới",
                    "shop_api_new",
                )
            ],
            [
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "shop_back_btn",
                    "⬅️ Quay lại",
                    "start:shop",
                )
            ]
        ]
    })
}

fn shop_product_list_payload(
    ctx: &AppContext,
    chat_id: ChatId,
    text: &str,
    reply_markup: Value,
) -> serde_json::Result<Value> {
    let mut payload =
        i18n::message_payload_with_json_keyboard(ctx, chat_id, "", text, reply_markup)?;
    let mut entities = payload
        .get("entities")
        .cloned()
        .map(serde_json::from_value::<Vec<MessageEntity>>)
        .transpose()?
        .unwrap_or_default();
    let rendered_text = payload.get("text").and_then(Value::as_str).unwrap_or("");
    entities.extend(shop_product_list_bold_entities(rendered_text));
    entities.extend(shop_product_list_price_entities(rendered_text));

    if let Some(obj) = payload.as_object_mut() {
        obj.remove("parse_mode");
        if entities.is_empty() {
            obj.remove("entities");
        } else {
            obj.insert("entities".to_string(), serde_json::to_value(entities)?);
        }
    }
    Ok(payload)
}

#[allow(clippy::too_many_arguments)]
async fn process_order(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    dialogue: BotDialogue,
    product_id: i64,
    qty: i64,
    customer_input: Option<String>,
    plan_id: Option<i64>,
) -> Result<()> {
    let lang = i18n::user_lang_by_id(&ctx, user_id).await;
    if user_id == 0 {
        ctx.bot
            .send_message(
                chat_id,
                tl(
                    &ctx,
                    &lang,
                    "user_unknown_retry",
                    "Cannot identify user. Please try again later.",
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }

    let product: Product = match repo::get_product(&ctx.pool, product_id).await? {
        Some(p) => p,
        None => {
            ctx.bot
                .send_message(
                    chat_id,
                    tl(&ctx, &lang, "not_found_product", "Product does not exist."),
                )
                .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                .await?;
            return Ok(());
        }
    };

    let delivery_type = orders_api::product_delivery_type(&product).to_string();
    let is_uploaded_file = delivery_type == "uploaded_file";
    let requires_input = delivery_type == "manual_input";

    if requires_input
        && customer_input
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        dialogue
            .update(State::CollectingInfo {
                product_id,
                qty,
                plan_id,
            })
            .await?;
        let prompt = product.input_prompt.clone().unwrap_or_else(|| {
            tl(
                &ctx,
                &lang,
                "default_input_prompt",
                "Enter email or activation information to complete:",
            )
        });
        ctx.bot
            .send_message(
                chat_id,
                trl(
                    &ctx,
                    &lang,
                    "info_prompt",
                    "📝 {prompt}\n\nPlease enter it accurately so the order can be processed.",
                    &[("prompt", prompt.clone())],
                ),
            )
            .reply_markup(shop_action_result_keyboard(&ctx, &lang))
            .await?;
        return Ok(());
    }
    let customer_input = customer_input.map(|s| s.trim().to_string());

    let mut plan_label = None;
    let mut plan_months = None;
    let mut plan_price = None;
    let uploaded_file_stock = if is_uploaded_file {
        repo::count_product_items(&ctx.pool, product_id)
            .await
            .unwrap_or(0)
    } else {
        0
    };
    let requested_qty = qty;
    let amount = if let Some(pid) = plan_id {
        if let Some(plan) = repo::get_product_plan(&ctx.pool, pid).await? {
            if plan.product_id != product.id {
                ctx.bot
                    .send_message(
                        chat_id,
                        tl(
                            &ctx,
                            &lang,
                            "plan_invalid_for_product",
                            "This plan is invalid for this product.",
                        ),
                    )
                    .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(());
            }
            plan_label = Some(plan.label.clone());
            plan_months = Some(plan.months);
            plan_price = Some(plan.price);
            // lock qty to plan months
            plan.price
        } else {
            ctx.bot
                .send_message(
                    chat_id,
                    tl(&ctx, &lang, "plan_not_found", "Plan does not exist."),
                )
                .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                .await?;
            return Ok(());
        }
    } else {
        product.price * requested_qty
    };
    let qty = order_qty_for_delivery_type(&delivery_type, requested_qty, plan_months);
    let memo = generate_memo(&ctx).await?;

    let mut order = Order::new(
        user_id,
        chat_id.0,
        product.id,
        qty,
        amount,
        memo.clone(),
        customer_input.clone(),
        plan_id,
        plan_label.clone(),
        plan_months,
        plan_price,
    );

    if delivery_type == "stock_item" || is_uploaded_file {
        let no_reserve_reason = no_reserve_reason_for_user(&ctx, user_id).await?;
        let stock = if is_uploaded_file {
            uploaded_file_stock
        } else {
            repo::count_product_items(&ctx.pool, product_id)
                .await
                .unwrap_or(0)
        };
        let (reservation_mode, risk_reason) =
            stock_backed_order_reservation_mode(stock, qty, no_reserve_reason);
        order.reservation_mode = reservation_mode;
        let mut tx = ctx.pool.begin().await?;
        repo::insert_order_tx(&mut tx, &order).await?;
        if let Some(reason) = risk_reason {
            let window_started_at =
                (Utc::now() - Duration::hours(NO_RESERVE_WINDOW_HOURS)).to_rfc3339();
            orders_repo::insert_order_risk_event(
                &mut tx,
                user_id,
                chat_id.0,
                "no_reserve_order",
                &reason,
                &window_started_at,
            )
            .await?;
            tx.commit().await?;
            notify_admins_for_no_reserve_order(&ctx, &order, &product, &reason).await;
        } else {
            tx.commit().await?;
        }
    } else {
        // Không dùng kho; lưu sẵn delivered_data để tránh lấy stock khi thanh toán.
        let info = customer_input.clone().unwrap_or_else(|| "N/A".to_string());
        let plan_desc = plan_label
            .as_ref()
            .map(|l| {
                trl(
                    &ctx,
                    &lang,
                    "plan_months_value",
                    "{label} ({months} months)",
                    &[
                        ("label", l.clone()),
                        ("months", plan_months.unwrap_or(qty).to_string()),
                    ],
                )
            })
            .unwrap_or_else(|| tl(&ctx, &lang, "plan_none", "No plan"));
        order.delivered_data = Some(format!("plan: {plan_desc}\ninfo: {info}"));
        let mut tx = ctx.pool.begin().await?;
        repo::insert_order_tx(&mut tx, &order).await?;
        tx.commit().await?;
    }

    let vietqr_url = vietqr_link(&ctx.bank_name(), &ctx.bank_account(), amount, &memo);
    let qr_url: Url = vietqr_url.parse()?;

    let info_line = customer_input
        .as_ref()
        .map(|v| {
            trl(
                &ctx,
                &lang,
                "order_info_line",
                "\n• Info: {info}",
                &[("info", v.clone())],
            )
        })
        .unwrap_or_default();
    let plan_line = if let Some(label) = &plan_label {
        let months = plan_months.unwrap_or(qty);
        trl(
            &ctx,
            &lang,
            "order_plan_line",
            "\n• Plan: {label} ({months} months)",
            &[("label", label.clone()), ("months", months.to_string())],
        )
    } else {
        "".to_string()
    };
    let unit_price = if plan_label.is_some() {
        plan_price.unwrap_or(amount)
    } else {
        product.price
    };
    let confirm_text = trl(
        &ctx,
        &lang,
        "order_confirm_text",
        "🛒 ORDER CONFIRMATION\n\n• Product: {product}{plan_line}\n• Quantity: {qty}\n• Unit price: {unit_price}\n• Total: {total}{info_line}\n\n⏳ Please pay using the instructions below.",
        &[
            ("product", product.name.clone()),
            ("plan_line", plan_line),
            ("qty", qty.to_string()),
            ("unit_price", format_vnd(unit_price)),
            ("total", format_vnd(amount)),
            ("info_line", info_line),
        ],
    );
    i18n::send_message_for_key(&ctx, chat_id, "order_confirm_text", confirm_text).await?;

    let bank_line = if let Some(name) = &ctx.bank_account_name() {
        format!("{} – {}", html_escape(&ctx.bank_name()), html_escape(name))
    } else {
        html_escape(&ctx.bank_name())
    };
    let pay_text = trl(
        &ctx,
        &lang,
        "order_pay_caption",
        "💰 Amount: {total}\n🏦 Bank: {bank_line}\n📱 Account: {acct}\n📝 Transfer memo: {memo}\n\n⚠️ Enter the exact memo so the system can process automatically.\n\n📸 Scan the QR or transfer manually.",
        &[
            ("total", format_vnd(amount)),
            ("bank_line", bank_line),
            ("acct", copyable_code(&ctx.bank_account())),
            ("memo", copyable_code(&memo)),
        ],
    );
    // Kiểm tra ví: nếu đủ số dư thì hiện nút thanh toán bằng ví
    let wallet_balance = wallet_repo::get_or_create_wallet(&ctx.pool, user_id)
        .await
        .map(|w| w.balance)
        .unwrap_or(0);

    let keyboard = build_checkout_keyboard(&ctx, &lang, &order.id, wallet_balance, amount);

    let expires_at = order_expires_at(&order.created_at);
    let initial_caption = render_qr_caption(
        &pay_text,
        &trl(
            &ctx,
            &lang,
            "qr_countdown",
            "⏰ QR valid for: {time}",
            &[("time", "05:00".to_string())],
        ),
    );
    let qr_message = ctx
        .bot
        .send_photo(chat_id, InputFile::url(qr_url))
        .caption(initial_caption)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard.clone())
        .await?;
    spawn_order_qr_countdown(
        ctx.clone(),
        chat_id,
        qr_message.id,
        order.id.clone(),
        pay_text,
        expires_at,
        keyboard,
        lang,
    );

    dialogue.update(State::Idle).await?;
    Ok(())
}

fn build_checkout_keyboard(
    ctx: &AppContext,
    lang: &str,
    order_id: &str,
    wallet_balance: i64,
    amount: i64,
) -> InlineKeyboardMarkup {
    let mut kb_rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    if wallet_balance >= amount {
        kb_rows.push(vec![InlineKeyboardButton::callback(
            trl(
                ctx,
                lang,
                "paywallet_btn",
                "💳 Pay with wallet ({balance})",
                &[("balance", format_vnd(wallet_balance))],
            ),
            format!("paywallet:{order_id}"),
        )]);
    }
    if ctx.binance_pay_enabled() {
        kb_rows.push(vec![i18n::inline_button_callback(
            ctx,
            lang,
            "pay_binance_btn",
            "🟡 Binance Pay",
            format!("cryptopay:binance:{order_id}"),
        )]);
    }
    if ctx.bep20_enabled() {
        kb_rows.push(vec![i18n::inline_button_callback(
            ctx,
            lang,
            "pay_bep20_btn",
            "🟢 USDT BEP20",
            format!("cryptopay:bep20:{order_id}"),
        )]);
    }
    kb_rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Wallet",
        "start:wallet",
    )]);
    kb_rows.push(vec![
        i18n::inline_button_callback(ctx, lang, "start_btn_shop", "🛒 Shop", "start:shop"),
        i18n::inline_button_callback(
            ctx,
            lang,
            "cancel_order_btn",
            "❌ Cancel order",
            format!("cancel:{order_id}"),
        ),
    ]);
    InlineKeyboardMarkup::new(kb_rows)
}

fn build_product_keyboard(
    ctx: &AppContext,
    lang: &str,
    products: &[(Product, i64)],
    page: i64,
    has_prev: bool,
    has_next: bool,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for (p, _stock) in products {
        rows.push(vec![InlineKeyboardButton::callback(
            product_button_label(p),
            format!("buy:{}", p.id),
        )]);
    }
    if has_prev || has_next {
        let mut nav = Vec::new();
        if has_prev {
            nav.push(i18n::inline_button_callback(
                ctx,
                lang,
                "pagination_prev",
                "◀️ Previous",
                format!("shop:{}", page - 1),
            ));
        }
        if has_next {
            nav.push(i18n::inline_button_callback(
                ctx,
                lang,
                "pagination_next",
                "Next ▶️",
                format!("shop:{}", page + 1),
            ));
        }
        rows.push(nav);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Wallet",
        "start:wallet",
    )]);
    InlineKeyboardMarkup::new(rows)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShopCategoryButton {
    label: String,
    emoji: Option<String>,
    custom_emoji_id: Option<String>,
}

fn build_shop_home_keyboard_json(
    ctx: &AppContext,
    lang: &str,
    categories: &[ShopCategoryButton],
    uncategorized_products: &[(Product, i64)],
) -> Value {
    let mut rows = Vec::new();
    for chunk in categories.chunks(3) {
        rows.push(chunk.iter().map(category_button_json).collect::<Vec<_>>());
    }
    for (product, _stock) in uncategorized_products {
        rows.push(vec![product_buy_button_json(product)]);
    }

    let mut action_row = vec![
        i18n::inline_button_callback_json(
            ctx,
            lang,
            "shop_btn_wallet",
            "👛 Ví của tôi",
            "start:wallet",
        ),
        i18n::inline_button_callback_json(
            ctx,
            lang,
            "shop_btn_help",
            "💬 Liên hệ admin",
            "start:help",
        ),
    ];
    let notification_url = ctx.get_text("required_channel_url", "").trim().to_string();
    if Url::parse(&notification_url).is_ok() {
        action_row.push(inline_button_url_json(
            ctx,
            "shop_btn_notifications",
            tl(ctx, lang, "shop_btn_notifications", "🔔 Thông báo"),
            notification_url,
        ));
    } else {
        action_row.push(i18n::inline_button_callback_json(
            ctx,
            lang,
            "shop_btn_notifications",
            "🔔 Thông báo",
            "start:help",
        ));
    }
    rows.push(action_row);
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "shop_btn_api_integration",
        "🔌 Tích hợp API",
        "shop_api",
    )]);
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "back_btn",
        "⬅️ Quay lại",
        "start:menu",
    )]);

    json!({ "inline_keyboard": rows })
}

fn category_button_json(category: &ShopCategoryButton) -> Value {
    let callback_data = format!("shop_cat:{}", category.label);
    if let Some(custom_id) = category.custom_emoji_id.as_deref() {
        json!({
            "text": category.label,
            "callback_data": callback_data,
            "icon_custom_emoji_id": custom_id,
        })
    } else if let Some(emoji) = category.emoji.as_deref() {
        json!({
            "text": format!("{} {}", emoji, category.label),
            "callback_data": callback_data,
        })
    } else {
        json!({
            "text": category.label,
            "callback_data": callback_data,
        })
    }
}

fn inline_button_url_json(
    ctx: &AppContext,
    key: &str,
    text: impl Into<String>,
    url: impl Into<String>,
) -> Value {
    let parts = i18n::button_parts_for_key(ctx, key, text);
    let mut button = json!({
        "text": parts.text,
        "url": url.into(),
    });
    if let Some(icon_id) = parts.icon_custom_emoji_id
        && let Some(obj) = button.as_object_mut()
    {
        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
    }
    button
}

fn build_category_product_keyboard_json(
    ctx: &AppContext,
    lang: &str,
    products: &[(Product, i64)],
) -> Value {
    let mut rows = Vec::new();
    for (product, _stock) in products {
        rows.push(vec![product_buy_button_json(product)]);
    }
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "shop_back_btn",
        "◀️ Quay lại",
        "start:shop",
    )]);

    json!({ "inline_keyboard": rows })
}

fn product_buy_button_json(product: &Product) -> Value {
    let (rendered_name, placeholder_custom_id) =
        render_button_custom_emoji_placeholders(&product.name);
    let product_name = truncate_button_text(&rendered_name, PRODUCT_BUTTON_NAME_MAX_CHARS);
    let text = format!("{} — {}", product_name, format_vnd(product.price));
    let callback_data = format!("buy:{}", product.id);
    if let Some(custom_id) =
        product_button_custom_emoji_id(product).or(placeholder_custom_id.as_deref())
    {
        json!({
            "text": text,
            "callback_data": callback_data,
            "icon_custom_emoji_id": custom_id,
        })
    } else {
        json!({
            "text": format!("{} {}", product_button_emoji(product), text),
            "callback_data": callback_data,
        })
    }
}

fn product_button_json(product: &Product) -> Value {
    let callback_data = format!("buy:{}", product.id);
    let (_, placeholder_custom_id) = render_button_custom_emoji_placeholders(&product.name);
    if let Some(custom_id) =
        product_button_custom_emoji_id(product).or(placeholder_custom_id.as_deref())
    {
        json!({
            "text": product_button_label(product),
            "callback_data": callback_data,
            "icon_custom_emoji_id": custom_id,
        })
    } else {
        json!({
            "text": product_button_label(product),
            "callback_data": callback_data,
        })
    }
}

fn product_button_label(product: &Product) -> String {
    let (rendered_name, placeholder_custom_id) =
        render_button_custom_emoji_placeholders(&product.name);
    let product_name = truncate_button_text(&rendered_name, PRODUCT_BUTTON_NAME_MAX_CHARS);
    if product_button_custom_emoji_id(product).is_some() || placeholder_custom_id.is_some() {
        product_name
    } else {
        format!("{} {}", product_button_emoji(product), product_name)
    }
}

fn format_product_list_text(
    ctx: &AppContext,
    lang: &str,
    products: &[(Product, i64)],
    _page: i64,
    _wallet_balance: i64,
) -> String {
    let mut lines = vec![
        tl(ctx, lang, "shop_list_title", "📋 MENU SẢN PHẨM"),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
        tl(
            ctx,
            lang,
            "shop_digital_warning",
            "🎁 NẠP VÍ BONUS 5-10% 🔥\n👇 CHỌN SẢN PHẨM BÊN DƯỚI:",
        ),
    ];

    let mut categories = Vec::new();
    for (product, _stock) in products {
        let category = product_category(product, ctx, lang);
        if !categories.iter().any(|existing| existing == &category) {
            categories.push(category);
        }
    }

    for category in categories {
        lines.push(String::new());
        lines.push(category.to_uppercase());

        for (product, stock) in products {
            if product_category(product, ctx, lang) != category {
                continue;
            }
            lines.push(format!(
                "• {} — {} ({})",
                product.name.trim(),
                format_vnd(product.price),
                product_stock_display(product, *stock, ctx, lang),
            ));
        }
    }

    lines.join("\n")
}

fn shop_product_list_bold_entities(text: &str) -> Vec<MessageEntity> {
    let mut entities = Vec::new();
    let mut offset = 0usize;
    for (idx, line) in text.split('\n').enumerate() {
        let len = line.encode_utf16().count();
        let is_bold_line = idx <= 1
            || (!line.is_empty()
                && !line.starts_with('•')
                && !line.starts_with("🎁")
                && !line.starts_with("👇"));
        if is_bold_line && len > 0 {
            entities.push(MessageEntity::bold(offset, len));
        }
        offset += len + 1;
    }
    entities
}

fn shop_product_list_price_entities(text: &str) -> Vec<MessageEntity> {
    let mut entities = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\n') {
        if line.starts_with('•')
            && let Some(price_start) = line.find(" — ").map(|idx| idx + " — ".len())
        {
            let price_end = line[price_start..]
                .find(" (")
                .map(|idx| price_start + idx)
                .unwrap_or(line.len());
            if price_end > price_start {
                let price_offset = offset + line[..price_start].encode_utf16().count();
                let price_len = line[price_start..price_end].encode_utf16().count();
                entities.push(MessageEntity::italic(price_offset, price_len));
            }
        }
        offset += line.encode_utf16().count() + 1;
    }
    entities
}

fn category_buttons_from_products(products: &[(Product, i64)]) -> Vec<ShopCategoryButton> {
    let mut categories = Vec::new();
    for (product, _stock) in products {
        let Some(category) = product
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !categories
            .iter()
            .any(|existing: &ShopCategoryButton| existing.label == category)
        {
            categories.push(ShopCategoryButton {
                label: category.to_string(),
                emoji: product
                    .category_emoji
                    .as_deref()
                    .or(product.button_emoji.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                custom_emoji_id: product
                    .category_custom_emoji_id
                    .as_deref()
                    .or(product.button_custom_emoji_id.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            });
        }
    }
    categories
}

fn uncategorized_products_from_products(products: &[(Product, i64)]) -> Vec<(Product, i64)> {
    products
        .iter()
        .filter(|(product, _stock)| {
            product
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        })
        .cloned()
        .collect()
}

fn product_category(product: &Product, ctx: &AppContext, lang: &str) -> String {
    let label = product
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| tl(ctx, lang, "shop_category_default", "SẢN PHẨM"));

    if let Some(custom_id) = product
        .category_custom_emoji_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("{{{custom_id}}} {label}");
    }

    if let Some(emoji) = product
        .category_emoji
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("{emoji} {label}");
    }

    label
}

fn product_button_emoji(product: &Product) -> &str {
    product
        .button_emoji
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("➖")
}

fn product_button_custom_emoji_id(product: &Product) -> Option<&str> {
    product
        .button_custom_emoji_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn product_stock_display(product: &Product, stock: i64, ctx: &AppContext, lang: &str) -> String {
    if orders_api::product_delivery_type(product) == "manual_input" {
        return tl(ctx, lang, "shop_stock_manual", "✅ có sẵn");
    }
    trl(
        ctx,
        lang,
        "shop_stock_auto",
        "còn {stock}",
        &[("stock", stock.max(0).to_string())],
    )
}

fn truncate_button_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let shortened = trimmed
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_string();
    format!("{shortened}...")
}

fn description_for_quantity_prompt(description: &str) -> String {
    let trimmed = description.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn product_description_prompt_line(
    ctx: &AppContext,
    lang: &str,
    product: &Product,
    sold_count: i64,
) -> String {
    let mut lines = Vec::new();
    if let Some(desc) = product
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(desc.to_string());
    }

    if product.show_sold_count.unwrap_or(0) != 0 {
        lines.push(trl(
            ctx,
            lang,
            "product_sold_count_line",
            "Đã bán: {sold} sản phẩm",
            &[("sold", sold_count.max(0).to_string())],
        ));
    }

    if lines.is_empty() {
        return String::new();
    }

    trl(
        ctx,
        lang,
        "product_description_line",
        "📝 Mô tả:\n{description}\n\n",
        &[("description", lines.join("\n\n"))],
    )
}

fn render_button_custom_emoji_placeholders(value: &str) -> (String, Option<String>) {
    let mut rendered = String::with_capacity(value.len());
    let mut first_custom_id = None;
    let mut byte_index = 0usize;
    while byte_index < value.len() {
        let remaining = &value[byte_index..];
        if let Some(rest) = remaining.strip_prefix('{')
            && let Some(close_index) = rest.find('}')
        {
            let candidate = &rest[..close_index];
            if (8..=64).contains(&candidate.len()) && candidate.chars().all(|c| c.is_ascii_digit())
            {
                rendered.push('✨');
                first_custom_id.get_or_insert_with(|| candidate.to_string());
                byte_index += close_index + 2;
                continue;
            }
        }
        if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }
    (rendered, first_custom_id)
}

fn uploaded_file_has_sellable_stock(_product: &Product, stock: i64) -> bool {
    stock > 0
}

fn product_is_available_for_purchase(product: &Product) -> bool {
    product.is_active.unwrap_or(1) != 0
}

fn uploaded_file_quantity_prompt(
    product_name: &str,
    price: i64,
    stock: i64,
    desc_text: Option<&str>,
    ctx: &AppContext,
    lang: &str,
) -> String {
    trl(
        ctx,
        lang,
        "uploaded_file_quantity_prompt",
        "✅ You selected {product} — {price}\n📦 File stock left: {stock}\n{description}📎 Product files will be sent automatically after payment.\n\n⌨️ Enter the number of files to buy:",
        &[
            ("product", product_name.to_string()),
            ("price", format_vnd(price)),
            ("stock", stock.to_string()),
            ("description", desc_text.unwrap_or("").to_string()),
        ],
    )
}

fn order_qty_for_delivery_type(
    delivery_type: &str,
    requested_qty: i64,
    plan_months: Option<i64>,
) -> i64 {
    if delivery_type == "manual_input" {
        plan_months.unwrap_or(requested_qty)
    } else {
        requested_qty
    }
}

fn quantity_keyboard(ctx: &AppContext, lang: &str, require_input: bool) -> InlineKeyboardMarkup {
    let values = if require_input {
        vec![1, 6, 12]
    } else {
        vec![1, 2, 3, 5, 10]
    };
    let buttons = values
        .into_iter()
        .map(|v| InlineKeyboardButton::callback(v.to_string(), format!("qty:{v}")))
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(vec![
        buttons,
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_wallet",
            "💳 Wallet",
            "start:wallet",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "back_btn",
            "⬅️ Back",
            "start:shop",
        )],
    ])
}

fn plan_keyboard(
    ctx: &AppContext,
    lang: &str,
    plans: &[crate::domains::products::models::ProductPlan],
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for chunk in plans.chunks(2) {
        let mut row = Vec::new();
        for p in chunk {
            let label = format!("{} - {}", p.label, format_vnd(p.price));
            row.push(InlineKeyboardButton::callback(
                label,
                format!("plan:{}", p.id),
            ));
        }
        rows.push(row);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Wallet",
        "start:wallet",
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "back_btn",
        "⬅️ Back",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn tl(ctx: &AppContext, lang: &str, key: &str, default: &str) -> String {
    i18n::t(ctx, lang, key, default)
}

fn is_shop_command(text: &str) -> bool {
    let command = text.split_whitespace().next().unwrap_or("");
    command == "/shop" || command.starts_with("/shop@")
}

fn is_any_bot_command(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|command| command.starts_with('/'))
}

fn trl(ctx: &AppContext, lang: &str, key: &str, default: &str, vars: &[(&str, String)]) -> String {
    i18n::tr(ctx, lang, key, default, vars)
}

fn format_vnd(amount: i64) -> String {
    let s = amount.abs().to_string();
    let mut with_sep = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            with_sep.push('.');
        }
        with_sep.push(ch);
    }
    let formatted: String = with_sep.chars().rev().collect();
    if amount < 0 {
        format!("-{}đ", formatted)
    } else {
        format!("{}đ", formatted)
    }
}

fn parse_reserved_ids(s: &str) -> Vec<i64> {
    s.split(',')
        .filter_map(|x| x.trim().parse::<i64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use rust_decimal::Decimal;
    use sqlx::sqlite::SqlitePoolOptions;

    fn test_ctx() -> Arc<AppContext> {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        test_ctx_with_pool(pool)
    }

    fn test_ctx_with_pool(sqlite_pool: sqlx::SqlitePool) -> Arc<AppContext> {
        AppContext::new(
            Bot::new("test-token"),
            sqlite_pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    #[test]
    fn shop_command_is_recognized_even_with_arguments_or_bot_username() {
        assert!(is_shop_command("/shop"));
        assert!(is_shop_command("/shop  "));
        assert!(is_shop_command("/shop page"));
        assert!(is_shop_command("/shop@zvw180bot"));
        assert!(!is_shop_command("/shopping"));
        assert!(!is_shop_command("1"));
    }

    #[test]
    fn bot_commands_are_recognized_so_state_handlers_can_defer_them() {
        assert!(is_any_bot_command("/orders"));
        assert!(is_any_bot_command("/wallet"));
        assert!(is_any_bot_command("/help something"));
        assert!(!is_any_bot_command("2"));
        assert!(!is_any_bot_command("hello"));
    }

    #[tokio::test]
    async fn quantity_keyboard_has_back_button_below_wallet() {
        let ctx = test_ctx();
        let keyboard = quantity_keyboard(&ctx, "vi", false);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[1][0]["text"], "💳 Wallet");
        assert_eq!(rows[2][0]["text"], "⬅️ Back");
        assert_eq!(rows[2][0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn long_product_names_are_shortened_in_product_buttons() {
        let product = Product {
            id: 42,
            name: "San pham co ten rat dai de test Telegram bi an nut inline khi hien thi"
                .to_string(),
            price: 5_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: Some("Tool".to_string()),
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: Some("🧩".to_string()),
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };

        let ctx = test_ctx();
        let keyboard = build_product_keyboard(&ctx, "vi", &[(product, 5)], 0, false, false);
        let json = serde_json::to_value(&keyboard).unwrap();
        let label = json["inline_keyboard"][0][0]["text"].as_str().unwrap();

        assert_eq!(label, "🧩 San pham co ten rat dai de test...");
        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], "buy:42");
    }

    #[tokio::test]
    async fn product_button_json_uses_custom_emoji_icon_when_configured() {
        let product = Product {
            id: 42,
            name: "CapCut Pro".to_string(),
            price: 5_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: Some("Tool".to_string()),
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: Some("🧩".to_string()),
            button_custom_emoji_id: Some("5368324170671202286".to_string()),
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };

        let button = product_button_json(&product);

        assert_eq!(button["text"], "CapCut Pro");
        assert_eq!(button["callback_data"], "buy:42");
        assert_eq!(button["icon_custom_emoji_id"], "5368324170671202286");
    }

    #[tokio::test]
    async fn product_button_json_uses_first_title_placeholder_as_icon() {
        let product = Product {
            id: 42,
            name: "CapCut {5375135722514685501} Pro".to_string(),
            price: 5_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: Some("Tool".to_string()),
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: Some("🧩".to_string()),
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };

        let button = product_button_json(&product);

        assert_eq!(button["text"], "CapCut ✨ Pro");
        assert_eq!(button["callback_data"], "buy:42");
        assert_eq!(button["icon_custom_emoji_id"], "5375135722514685501");
    }

    #[tokio::test]
    async fn product_list_text_groups_products_by_category_with_requested_stock_labels() {
        let products = vec![
            (
                Product {
                    id: 1,
                    name: "Gemini Pro + 5TB Pixel mail".to_string(),
                    price: 35_000,
                    is_active: Some(1),
                    requires_input: Some(0),
                    input_prompt: None,
                    description: None,
                    image_url: None,
                    delivery_type: Some("stock_item".to_string()),
                    file_path: None,
                    file_name: None,
                    file_mime: None,
                    category_id: None,
                    category: Some("Gemini Pro".to_string()),
                    category_emoji: None,
                    category_custom_emoji_id: None,
                    button_emoji: Some("1️⃣".to_string()),
                    button_custom_emoji_id: None,
                    created_at: None,
                    sort_order: None,
                    show_sold_count: Some(0),
                },
                2,
            ),
            (
                Product {
                    id: 2,
                    name: "Meitu SVIP".to_string(),
                    price: 70_000,
                    is_active: Some(1),
                    requires_input: Some(1),
                    input_prompt: None,
                    description: None,
                    image_url: None,
                    delivery_type: Some("manual_input".to_string()),
                    file_path: None,
                    file_name: None,
                    file_mime: None,
                    category_id: None,
                    category: Some("Meitu".to_string()),
                    category_emoji: None,
                    category_custom_emoji_id: None,
                    button_emoji: Some("Ⓜ️".to_string()),
                    button_custom_emoji_id: None,
                    created_at: None,
                    sort_order: None,
                    show_sold_count: Some(0),
                },
                1,
            ),
        ];
        let ctx = test_ctx();
        let text = format_product_list_text(&ctx, "vi", &products, 0, 0);

        assert!(text.contains("📋 MENU SẢN PHẨM"));
        assert!(text.contains("━━━━━━━━━━━━━━━━━━━━"));
        assert!(text.contains("GEMINI PRO"));
        assert!(text.contains("• Gemini Pro + 5TB Pixel mail — 35.000đ (còn 2)"));
        assert!(text.contains("MEITU"));
        assert!(text.contains("• Meitu SVIP — 70.000đ (✅ có sẵn)"));
        assert!(!text.contains("💵 Số dư"));
    }

    #[tokio::test]
    async fn product_category_uses_managed_custom_emoji_for_heading() {
        let product = Product {
            id: 1,
            name: "CapCut Vip".to_string(),
            price: 5_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: Some(7),
            category: Some("CAP CUT".to_string()),
            category_emoji: Some("🎬".to_string()),
            category_custom_emoji_id: Some("5375135722514685501".to_string()),
            button_emoji: None,
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };
        let ctx = test_ctx();

        assert_eq!(
            product_category(&product, &ctx, "vi"),
            "{5375135722514685501} CAP CUT"
        );
    }

    #[tokio::test]
    async fn shop_product_list_payload_italicizes_product_prices() {
        let ctx = test_ctx();
        let text = "📋 MENU SẢN PHẨM\n━━━━━━━━━━━━━━━━━━━━\n\nPLUS\n• Plus — 2.000đ (còn 5)";
        let payload =
            shop_product_list_payload(&ctx, ChatId(1), text, json!({"inline_keyboard":[]}))
                .unwrap();
        let entities = payload["entities"].as_array().unwrap();
        let price_offset = text.find("2.000đ").unwrap();

        assert!(entities.iter().any(|entity| {
            entity["type"] == "italic"
                && entity["offset"] == text[..price_offset].encode_utf16().count()
                && entity["length"] == "2.000đ".encode_utf16().count()
        }));
    }

    #[tokio::test]
    async fn product_prompt_text_places_title_and_description_custom_emoji_placeholders() {
        let ctx = test_ctx();
        let desc = trl(
            &ctx,
            "vi",
            "product_description_line",
            "📝 Description:\n{description}\n\n",
            &[("description", "Mo ta {5420147074266044260}".to_string())],
        );
        let text = uploaded_file_quantity_prompt(
            "Test {5375135722514685501}",
            2_000,
            5,
            Some(&desc),
            &ctx,
            "vi",
        );
        let rich = i18n::rich_text_for_key(&ctx, "", text);

        assert!(rich.text.contains("Test ✨"));
        assert!(rich.text.contains("Mo ta ✨"));
        assert!(!rich.text.contains("{5375135722514685501}"));
        assert!(!rich.text.contains("{5420147074266044260}"));
        assert_eq!(rich.entities.len(), 2);
    }

    #[tokio::test]
    async fn shop_product_list_payload_keeps_bold_and_multiple_custom_emoji_entities() {
        let mut configs = std::collections::HashMap::new();
        configs.insert("telegram_i18n_emojis_enabled".to_string(), "1".to_string());
        configs.insert(
            "telegram_custom_emojis".to_string(),
            r#"{"📋":"5368324170671202286","🎁":"5368324170671202287","🔥":"5368324170671202288","✅":"5368324170671202289"}"#.to_string(),
        );
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            configs,
            BotTexts::default(),
            vec![],
        );

        let text = "📋 MENU SẢN PHẨM\n━━━━━━━━━━━━━━━━━━━━\n🎁 Bonus 🔥\n\nGEMINI PRO\n• Gói — 1.000đ (✅ có sẵn)";
        let payload =
            shop_product_list_payload(&ctx, ChatId(1), text, json!({"inline_keyboard":[]}))
                .unwrap();
        let entities = payload["entities"].as_array().unwrap();
        let entity_types = entities
            .iter()
            .filter_map(|entity| entity["type"].as_str())
            .collect::<Vec<_>>();

        assert!(!payload.as_object().unwrap().contains_key("parse_mode"));
        assert!(entity_types.iter().filter(|kind| **kind == "bold").count() >= 3);
        assert!(
            entity_types
                .iter()
                .filter(|kind| **kind == "custom_emoji")
                .count()
                >= 4
        );
    }

    #[tokio::test]
    async fn shop_product_list_payload_places_raw_custom_emoji_id_before_category_heading() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            std::collections::HashMap::from([(
                "telegram_i18n_emojis_enabled".to_string(),
                "1".to_string(),
            )]),
            BotTexts::default(),
            vec![],
        );

        let text = "📋 MENU SẢN PHẨM\n━━━━━━━━━━━━━━━━━━━━\n\n{5375135722514685501}PLUS\n• Plus — 2.000đ (còn 5)";
        let payload =
            shop_product_list_payload(&ctx, ChatId(1), text, json!({"inline_keyboard":[]}))
                .unwrap();
        let rendered = payload["text"].as_str().unwrap();
        let entities = payload["entities"].as_array().unwrap();

        assert!(rendered.contains("✨PLUS"));
        assert!(!rendered.contains("{5375135722514685501}"));
        assert!(entities.iter().any(|entity| {
            entity["type"] == "custom_emoji"
                && entity["custom_emoji_id"] == "5375135722514685501"
                && entity["offset"]
                    == "📋 MENU SẢN PHẨM\n━━━━━━━━━━━━━━━━━━━━\n\n"
                        .encode_utf16()
                        .count()
        }));
    }

    #[tokio::test]
    async fn shop_home_keyboard_uses_dynamic_category_buttons_and_action_row() {
        let mut configs = std::collections::HashMap::new();
        configs.insert(
            "required_channel_url".to_string(),
            "https://t.me/announcements".to_string(),
        );
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            configs,
            BotTexts::default(),
            vec![],
        );

        let keyboard = build_shop_home_keyboard_json(
            &ctx,
            "vi",
            &[
                ShopCategoryButton {
                    label: "Cursor".to_string(),
                    emoji: Some("◼️".to_string()),
                    custom_emoji_id: Some("5368324170671202286".to_string()),
                },
                ShopCategoryButton {
                    label: "Gemini Pro".to_string(),
                    emoji: Some("1️⃣".to_string()),
                    custom_emoji_id: None,
                },
                ShopCategoryButton {
                    label: "Meitu".to_string(),
                    emoji: Some("Ⓜ️".to_string()),
                    custom_emoji_id: None,
                },
                ShopCategoryButton {
                    label: "Claude Team".to_string(),
                    emoji: None,
                    custom_emoji_id: None,
                },
            ],
            &[],
        );
        let rows = keyboard["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0].as_array().unwrap().len(), 3);
        assert_eq!(rows[0][0]["text"], "Cursor");
        assert_eq!(rows[0][0]["callback_data"], "shop_cat:Cursor");
        assert_eq!(rows[0][0]["icon_custom_emoji_id"], "5368324170671202286");
        assert_eq!(rows[0][1]["text"], "1️⃣ Gemini Pro");
        assert_eq!(rows[0][1]["callback_data"], "shop_cat:Gemini Pro");
        assert_eq!(rows[1].as_array().unwrap().len(), 1);
        assert_eq!(rows[2].as_array().unwrap().len(), 3);
        assert_eq!(rows[2][0]["text"], "👛 Ví của tôi");
        assert_eq!(rows[2][0]["callback_data"], "start:wallet");
        assert_eq!(rows[2][1]["text"], "💬 Liên hệ admin");
        assert_eq!(rows[2][1]["callback_data"], "start:help");
        assert_eq!(rows[2][2]["text"], "🔔 Thông báo");
        assert_eq!(rows[2][2]["url"], "https://t.me/announcements");
        assert_eq!(rows[3][0]["text"], "🔌 Tích hợp API");
        assert_eq!(rows[3][0]["callback_data"], "shop_api");
    }

    #[tokio::test]
    async fn shop_home_keyboard_lists_uncategorized_products_as_buy_buttons() {
        let product = Product {
            id: 42,
            name: "Gói lẻ".to_string(),
            price: 88_888,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: None,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: Some("🎁".to_string()),
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };
        let ctx = test_ctx();
        let keyboard = build_shop_home_keyboard_json(&ctx, "vi", &[], &[(product, 0)]);
        let rows = keyboard["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["text"], "🎁 Gói lẻ — 88.888đ");
        assert_eq!(rows[0][0]["callback_data"], "buy:42");
        assert_eq!(rows[1][0]["callback_data"], "start:wallet");
    }

    #[tokio::test]
    async fn api_integration_page_shows_token_url_endpoints_and_usage() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: Some("https://shop.example".to_string()),
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        let text = format_api_integration_text(&ctx, 42, "abc-token");

        assert!(text.contains("Tích hợp API"));
        assert!(text.contains("<code>42:abc-token</code>"));
        assert!(text.contains("<code>https://shop.example/api/client/products</code>"));
        assert!(text.contains("Authorization: Bearer 42:abc-token"));
        assert!(text.contains("POST /api/client/orders"));
        assert!(text.contains("\"product_id\": 1"));
    }

    #[tokio::test]
    async fn api_integration_page_uses_runtime_base_url_config() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let mut configs = std::collections::HashMap::new();
        configs.insert(
            "base_url".to_string(),
            "https://runtime-shop.example/".to_string(),
        );
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            configs,
            BotTexts::default(),
            vec![],
        );

        let text = format_api_integration_text(&ctx, 42, "abc-token");

        assert!(text.contains("<code>https://runtime-shop.example</code>"));
        assert!(text.contains("<code>https://runtime-shop.example/api/client/products</code>"));
        assert!(!text.contains("localhost"));
    }

    #[tokio::test]
    async fn api_integration_page_falls_back_to_env_base_url_when_runtime_base_url_is_blank() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let mut configs = std::collections::HashMap::new();
        configs.insert("base_url".to_string(), "".to_string());
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: Some("https://env-shop.example".to_string()),
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            configs,
            BotTexts::default(),
            vec![],
        );

        let text = format_api_integration_text(&ctx, 42, "abc-token");

        assert!(text.contains("<code>https://env-shop.example/api/client/products</code>"));
        assert!(!text.contains("localhost"));
    }

    #[tokio::test]
    async fn api_integration_keyboard_has_rotate_and_back_buttons() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        let keyboard = api_integration_keyboard_json(&ctx, "vi");
        let rows = keyboard["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["callback_data"], "shop_api_new");
        assert_eq!(rows[1][0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn category_keyboard_lists_buy_buttons_and_back_to_shop() {
        let product = Product {
            id: 42,
            name: "Gói 1".to_string(),
            price: 60_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: Some("Gemini Pro".to_string()),
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: Some("1️⃣".to_string()),
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };
        let ctx = test_ctx();
        let keyboard = build_category_product_keyboard_json(&ctx, "vi", &[(product, 4)]);
        let rows = keyboard["inline_keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["text"], "1️⃣ Gói 1 — 60.000đ");
        assert_eq!(rows[0][0]["callback_data"], "buy:42");
        assert_eq!(rows[1][0]["text"], "◀️ Quay lại");
        assert_eq!(rows[1][0]["callback_data"], "start:shop");
    }

    #[test]
    fn copyable_code_escapes_html_for_telegram_caption() {
        assert_eq!(
            copyable_code("VCB <main> & owner"),
            "<code>VCB &lt;main&gt; &amp; owner</code>"
        );
    }

    #[test]
    fn product_image_path_candidates_prefer_persistent_upload_storage() {
        assert_eq!(
            product_image_path_candidates("/uploads/product_2.jpeg"),
            vec![
                "storage/uploads/product_2.jpeg".to_string(),
                "public/uploads/product_2.jpeg".to_string(),
            ]
        );
    }

    #[test]
    fn countdown_caption_edit_keeps_inline_keyboard() {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("🛒 Mua hàng", "start:shop"),
            InlineKeyboardButton::callback("❌ Hủy đơn hàng", "cancel:ABC"),
        ]]);
        let payload = keep_qr_keyboard_on_caption_edit(
            teloxide::payloads::EditMessageCaption::new(ChatId(1), MessageId(2))
                .caption("caption")
                .parse_mode(ParseMode::Html),
            &keyboard,
        );
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(
            json["reply_markup"],
            serde_json::to_value(&keyboard).unwrap()
        );
    }

    #[test]
    fn uploaded_file_order_qty_uses_requested_qty() {
        assert_eq!(order_qty_for_delivery_type("uploaded_file", 4, None), 4);
    }

    #[test]
    fn uploaded_file_product_is_sellable_only_when_file_stock_exists() {
        let product = Product {
            id: 1,
            name: "File".to_string(),
            price: 10_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("uploaded_file".to_string()),
            file_path: Some("storage/product_files/legacy.zip".to_string()),
            file_name: Some("legacy.zip".to_string()),
            file_mime: None,
            category_id: None,
            category: None,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: None,
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };

        assert!(!uploaded_file_has_sellable_stock(&product, 0));
        assert!(uploaded_file_has_sellable_stock(&product, 1));
    }

    #[test]
    fn inactive_product_is_not_available_for_purchase() {
        let mut product = Product {
            id: 42,
            name: "Old package".to_string(),
            price: 10_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: None,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: None,
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        };
        assert!(product_is_available_for_purchase(&product));

        product.is_active = Some(0);

        assert!(!product_is_available_for_purchase(&product));
    }

    #[tokio::test]
    async fn uploaded_file_quantity_prompt_mentions_stock_and_quantity() {
        let ctx = test_ctx();
        let text = uploaded_file_quantity_prompt("Tool", 50_000, 3, None, &ctx, "vi");

        assert!(text.contains("File stock left: 3"));
        assert!(text.contains("Enter the number of files to buy"));
        assert!(text.contains("Tool"));
        assert!(text.contains("50.000đ"));
    }

    #[test]
    fn quantity_prompt_description_keeps_single_blank_line_before_input() {
        let desc = "📝 Mô tả:\nTest bot\n\n";
        let text = format!(
            "✅ Bạn chọn Tool - 10.000đ\n📦 Còn lại: 1\n{}{}\n\n⌨️ Nhập số lượng muốn mua:",
            description_for_quantity_prompt(desc),
            ""
        );

        assert!(text.contains("Test bot\n\n⌨️ Nhập số lượng muốn mua:"));
        assert!(!text.contains("Test bot\n\n\n⌨️"));
    }

    fn test_product_with_description(description: Option<&str>) -> Product {
        Product {
            id: 42,
            name: "Tool".to_string(),
            price: 50_000,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: description.map(ToOwned::to_owned),
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: None,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: None,
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(1),
        }
    }

    #[tokio::test]
    async fn product_description_prompt_appends_sold_count_at_the_end_when_enabled() {
        let ctx = test_ctx();
        let product = test_product_with_description(Some("Mô tả"));

        let text = product_description_prompt_line(&ctx, "vi", &product, 12);

        assert!(text.contains("Mô tả"));
        assert!(text.trim_end().ends_with("Đã bán: 12 sản phẩm"));
    }

    #[tokio::test]
    async fn product_description_prompt_omits_sold_count_when_disabled() {
        let ctx = test_ctx();
        let mut product = test_product_with_description(Some("Mô tả"));
        product.show_sold_count = Some(0);

        let text = product_description_prompt_line(&ctx, "vi", &product, 12);

        assert!(!text.contains("Đã bán:"));
    }

    #[tokio::test]
    async fn checkout_keyboard_hides_crypto_buttons_when_disabled() {
        let ctx = test_ctx();
        let keyboard = build_checkout_keyboard(&ctx, "en", "order-1", 0, 50_000);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(!callbacks.iter().any(|data| data.starts_with("cryptopay:")));
        assert!(callbacks.contains(&"start:wallet"));
    }

    #[tokio::test]
    async fn checkout_keyboard_adds_bep20_button_when_enabled() {
        let mut config = Config {
            telegram_token: "test-token".to_string(),
            database_url: "sqlite::memory:".to_string(),
            bank_name: "VCB".to_string(),
            bank_account: Some("123".to_string()),
            bank_account_name: None,
            webhook_secret: "secret".to_string(),
            admin_jwt_secret: "12345678901234567890123456789012".to_string(),
            admin_setup_code: "setupcode".to_string(),
            admin_cookie_secure: false,
            base_url: None,
            i18n_dir: "i18n".to_string(),
            port: 8080,
            crypto: crate::config::CryptoConfig::default(),
        };
        config.crypto.bep20.enabled = true;
        config.crypto.bep20.merchant_wallet =
            Some("0x0000000000000000000000000000000000000001".to_string());
        config.crypto.bep20.bscscan_api_key = Some("bsc-key".to_string());
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        let keyboard = build_checkout_keyboard(&ctx, "en", "order-1", 0, 50_000);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"cryptopay:bep20:order-1"));
    }

    #[tokio::test]
    async fn shop_home_keyboard_has_back_to_main_menu() {
        let ctx = test_ctx();
        let keyboard = build_shop_home_keyboard_json(&ctx, "vi", &[], &[]);
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["callback_data"], "start:menu");
    }

    #[tokio::test]
    async fn crypto_payment_keyboard_has_back_to_shop() {
        let ctx = test_ctx();
        let keyboard = build_crypto_payment_keyboard(&ctx, "vi", 9);
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:shop"));
    }

    #[tokio::test]
    async fn crypto_action_result_keyboard_has_back_to_shop() {
        let ctx = test_ctx();
        let keyboard = crypto_action_result_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:shop"));
        assert!(callbacks.contains(&"start:wallet"));
    }

    #[tokio::test]
    async fn shop_action_result_keyboard_has_back_to_shop_and_wallet() {
        let ctx = test_ctx();
        let keyboard = shop_action_result_keyboard(&ctx, "vi");
        let json = serde_json::to_value(&keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();

        assert!(callbacks.contains(&"start:shop"));
        assert!(callbacks.contains(&"start:wallet"));
    }

    #[test]
    fn legacy_continue_shopping_callback_is_routed_to_shop() {
        assert!(is_shop_callback_data("shopnew:0"));
    }

    #[test]
    fn stock_notification_disable_callback_is_not_routed_to_shop_plugin() {
        assert!(!is_shop_callback_data("stocknotify:off"));
    }

    #[test]
    fn product_list_edit_failure_from_document_falls_back_to_new_message() {
        let err = anyhow::anyhow!(
            "{}",
            r#"Telegram editMessageText failed: {"ok":false,"description":"Bad Request: there is no text in the message to edit"}"#
        );

        assert!(should_fallback_to_new_product_list_message(&err));
    }

    async fn seed_reserved_order_crypto_payment(
        status: Option<CryptoPaymentStatus>,
    ) -> (
        sqlx::SqlitePool,
        Arc<AppContext>,
        Order,
        i64,
        CryptoPaymentRequest,
    ) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let ctx = test_ctx_with_pool(pool.clone());
        let product = repo::insert_product(
            &pool,
            "Key",
            10_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let item_id =
            sqlx::query("INSERT INTO product_items (product_id, content, is_buy) VALUES (?, ?, 1)")
                .bind(product.id)
                .bind("secret-key")
                .execute(&pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let order = Order {
            id: "ORDER-USDT-CANCEL".to_string(),
            user_id: 77,
            chat_id: 88,
            product_id: product.id,
            qty: 1,
            amount: 10_000,
            status: OrderStatus::Pending,
            bank_memo: "ORDER-USDT-CANCEL".to_string(),
            created_at: Utc::now().to_rfc3339(),
            paid_at: None,
            payment_tx_id: None,
            delivered_data: Some("secret-key".to_string()),
            reserved_item_ids: Some(item_id.to_string()),
            customer_input: None,
            plan_id: None,
            plan_label: None,
            plan_months: None,
            plan_price: None,
            reservation_mode: OrderReservationMode::Reserved,
        };
        orders_repo::insert_order(&pool, &order).await.unwrap();
        let payment = crypto_repo::create_crypto_payment(
            &pool,
            crypto_repo::NewCryptoPayment {
                purpose: "order".to_string(),
                order_id: Some(order.id.clone()),
                wallet_topup_id: None,
                user_id: order.user_id,
                chat_id: order.chat_id,
                method: CryptoPaymentMethod::Bep20,
                amount_vnd: order.amount,
                rate_vnd_per_usdt: Decimal::new(25_000, 0),
                amount_usdt_base: Decimal::new(4, 1),
                amount_usdt_expected: Decimal::new(400_001, 6),
                amount_token_units: "400001".to_string(),
                memo: "USDT-CANCEL".to_string(),
                address: Some("0x0000000000000000000000000000000000000001".to_string()),
                binance_prepay_id: None,
                binance_checkout_url: None,
                binance_qrcode_link: None,
                binance_qr_content: None,
                binance_deeplink: None,
                binance_universal_url: None,
                expires_at: Utc::now().to_rfc3339(),
            },
        )
        .await
        .unwrap();
        if let Some(status) = status {
            sqlx::query("UPDATE crypto_payment_requests SET status = ? WHERE id = ?")
                .bind(status.to_string())
                .bind(payment.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let payment = crypto_repo::find_crypto_payment_by_id(&pool, payment.id)
            .await
            .unwrap()
            .unwrap();

        (pool, ctx, order, item_id, payment)
    }

    #[tokio::test]
    async fn crypto_cancel_releases_reserved_order_stock_while_payment_pending() {
        let (pool, ctx, order, item_id, payment) = seed_reserved_order_crypto_payment(None).await;

        let cancelled = cancel_order_for_crypto_payment(&ctx, &payment)
            .await
            .unwrap();

        assert!(cancelled);
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Cancel);
        assert_eq!(updated.delivered_data, None);
        assert_eq!(updated.reserved_item_ids, None);
        let payment = crypto_repo::find_crypto_payment_by_id(&pool, payment.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(payment.status, CryptoPaymentStatus::Expired);
        let is_buy: i64 = sqlx::query_scalar("SELECT is_buy FROM product_items WHERE id = ?")
            .bind(item_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(is_buy, 0);
    }

    #[tokio::test]
    async fn crypto_cancel_releases_reserved_order_stock_even_after_payment_expired() {
        let (pool, ctx, order, item_id, payment) =
            seed_reserved_order_crypto_payment(Some(CryptoPaymentStatus::Expired)).await;

        let cancelled = cancel_order_for_crypto_payment(&ctx, &payment)
            .await
            .unwrap();

        assert!(cancelled);
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Cancel);
        assert_eq!(updated.delivered_data, None);
        assert_eq!(updated.reserved_item_ids, None);
        let is_buy: i64 = sqlx::query_scalar("SELECT is_buy FROM product_items WHERE id = ?")
            .bind(item_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(is_buy, 0);
    }

    #[tokio::test]
    async fn generated_order_memo_uses_configured_prefix_and_random_length() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            std::collections::HashMap::from([
                ("order_memo_prefix".to_string(), "SHOP".to_string()),
                ("order_memo_length".to_string(), "12".to_string()),
            ]),
            BotTexts::default(),
            vec![],
        );

        let memo = generate_memo(&ctx).await.unwrap();

        assert!(memo.starts_with("SHOP"));
        assert_eq!(memo.len(), "SHOP".len() + 12);
        assert!(memo.chars().all(|ch| ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn no_reserve_decision_flags_repeated_unpaid_orders() {
        let summary = orders_repo::OrderRiskSummary {
            total_orders: 4,
            unpaid_orders: 3,
        };

        let decision = no_reserve_decision(summary);

        assert!(decision.is_some());
        assert!(decision.unwrap().contains("3/4"));
    }

    #[test]
    fn no_reserve_decision_keeps_low_risk_users_reserved() {
        let low_count = orders_repo::OrderRiskSummary {
            total_orders: 2,
            unpaid_orders: 2,
        };
        let low_ratio = orders_repo::OrderRiskSummary {
            total_orders: 10,
            unpaid_orders: 6,
        };

        assert!(no_reserve_decision(low_count).is_none());
        assert!(no_reserve_decision(low_ratio).is_none());
    }

    #[test]
    fn stock_backed_orders_are_no_reserve_even_when_stock_is_short() {
        let (available_mode, available_reason) = stock_backed_order_reservation_mode(1, 1, None);
        let (short_mode, short_reason) = stock_backed_order_reservation_mode(0, 2, None);

        assert_eq!(available_mode, OrderReservationMode::NoReserve);
        assert_eq!(available_reason, None);
        assert_eq!(short_mode, OrderReservationMode::NoReserve);
        assert_eq!(short_reason, None);
    }

    #[tokio::test]
    async fn wallet_payment_rolls_back_stock_when_debit_fails() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let product = repo::insert_product(
            &pool,
            "Key",
            10_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            Some("stock_item"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let item_id =
            sqlx::query("INSERT INTO product_items (product_id, content, is_buy) VALUES (?, ?, 0)")
                .bind(product.id)
                .bind("secret-key")
                .execute(&pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let order = Order::new(
            42,
            420,
            product.id,
            1,
            10_000,
            "ORDER-WALLET-FAIL".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        orders_repo::insert_order(&pool, &order).await.unwrap();
        let owp = orders_repo::get_order_with_product(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();

        let err = complete_wallet_payment_transaction(&pool, &owp, 42, &order.id)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Số dư ví không đủ"));
        let is_buy: i64 = sqlx::query_scalar("SELECT is_buy FROM product_items WHERE id = ?")
            .bind(item_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(is_buy, 0);
        let updated = orders_repo::get_order(&pool, &order.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, OrderStatus::Pending);
        assert_eq!(updated.delivered_data, None);
        assert_eq!(updated.reserved_item_ids, None);
    }
}

async fn generate_memo(ctx: &AppContext) -> Result<String> {
    let prefix = ctx.order_memo_prefix();
    let random_len = ctx.order_memo_length();

    for _ in 0..5 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_alphanumeric())
            .map(char::from)
            .take(random_len)
            .collect::<String>()
            .to_uppercase();
        let memo = format!("{prefix}{suffix}");
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM orders WHERE bank_memo = ?")
                .bind(&memo)
                .fetch_one(&ctx.pool)
                .await
                .unwrap_or(0);
        if exists == 0 {
            return Ok(memo);
        }
    }
    Err(anyhow!("Không tạo được memo unique"))
}

pub struct ShopCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for ShopCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdShop"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "shop".to_string(),
            description: "View products".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("");
        let lang = if let Some(user) = msg.from() {
            i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            "en".to_string()
        };

        if is_shop_command(text) {
            send_products(ctx.clone(), ctx.bot.clone(), msg.chat.id, 0, None, &lang).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        if is_any_bot_command(text) {
            return Ok(false);
        }

        let state = dialogue.get().await?.unwrap_or_default();
        match state {
            State::ChoosingQty { product_id } => {
                handle_qty_message(ctx.clone(), msg, dialogue, product_id).await?;
                return Ok(true);
            }
            State::SelectingPlan { .. } => {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        tl(
                            &ctx,
                            &lang,
                            "fallback_plan",
                            "📅 Choose a plan/month using the buttons below to continue.",
                        ),
                    )
                    .reply_markup(shop_action_result_keyboard(&ctx, &lang))
                    .await?;
                return Ok(true);
            }
            State::CollectingInfo {
                product_id,
                qty,
                plan_id,
            } => {
                handle_info_message(ctx.clone(), msg, dialogue, product_id, qty, plan_id).await?;
                return Ok(true);
            }
            State::Idle => {}
            State::TopupEnterAmount => {}
            State::TopupUsdtEnterAmount => {}
            State::TopupBinanceEnterAmount => {}
        }

        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = q.data.clone().unwrap_or_default();
        if is_shop_callback_data(&text) {
            shop_handle_callback(ctx, q, dialogue).await?;
            return Ok(true);
        }
        Ok(false)
    }
}

fn is_shop_callback_data(text: &str) -> bool {
    text.starts_with("start:shop")
        || text.starts_with("shop:")
        || text.starts_with("shopnew:")
        || text == "shop_api"
        || text == "shop_api_new"
        || text.starts_with("shop_cat:")
        || text.starts_with("buy:")
        || text.starts_with("qty:")
        || text.starts_with("plan:")
        || text.starts_with("cancel:")
        || text.starts_with("paywallet:")
        || text.starts_with("cryptopay:")
}
