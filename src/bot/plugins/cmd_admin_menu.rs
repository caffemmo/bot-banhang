use std::sync::Arc;

use anyhow::Result;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode,
};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, i18n};

pub struct AdminMenuCommandPlugin;

const ADMIN_MENU_PREFIX: &str = "admin_menu:";
const ADMIN_MENU_HOME: &str = "admin_menu:home";
const ADMIN_MENU_ALL: &str = "admin_menu:all";
const ADMIN_MENU_BROADCAST: &str = "admin_menu:broadcast";
const ADMIN_MENU_AFFILIATE: &str = "admin_menu:affiliate";
const ADMIN_MENU_GROUP: &str = "admin_menu:group";
const ADMIN_MENU_REFUND: &str = "admin_menu:refund";

#[async_trait::async_trait]
impl AppPlugin for AdminMenuCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdAdminMenu"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "admin".to_string(),
            description: "Admin menu".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_command(text, "/admin") {
            return Ok(false);
        }

        let Some(user) = msg.from() else {
            return Ok(true);
        };
        if !is_admin_user(&ctx, user.id.0 as i64) {
            let lang = i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await;
            ctx.bot
                .send_message(
                    msg.chat.id,
                    i18n::t(&ctx, &lang, "unauthorized", "Unauthorized."),
                )
                .await?;
            return Ok(true);
        }

        send_admin_menu(&ctx, msg.chat.id).await?;
        Ok(true)
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
        if !data.starts_with(ADMIN_MENU_PREFIX) {
            return Ok(false);
        }

        if !is_admin_user(&ctx, q.from.id.0 as i64) {
            let _ = ctx
                .bot
                .answer_callback_query(q.id.clone())
                .text("Bạn không có quyền admin.")
                .show_alert(true)
                .await;
            return Ok(true);
        }

        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };

        let (text, keyboard) = match data.as_str() {
            ADMIN_MENU_HOME => (admin_home_text(), admin_home_keyboard()),
            ADMIN_MENU_ALL => (admin_all_commands_text(), admin_back_keyboard()),
            ADMIN_MENU_BROADCAST => (admin_broadcast_text(), admin_back_keyboard()),
            ADMIN_MENU_AFFILIATE => (admin_affiliate_text(), admin_back_keyboard()),
            ADMIN_MENU_GROUP => (admin_group_text(), admin_back_keyboard()),
            ADMIN_MENU_REFUND => (admin_refund_text(), admin_back_keyboard()),
            _ => (admin_home_text(), admin_home_keyboard()),
        };

        ctx.bot
            .send_message(msg.chat().id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
        Ok(true)
    }
}

async fn send_admin_menu(ctx: &AppContext, chat_id: teloxide::types::ChatId) -> Result<()> {
    ctx.bot
        .send_message(chat_id, admin_home_text())
        .parse_mode(ParseMode::Html)
        .reply_markup(admin_home_keyboard())
        .await?;
    Ok(())
}

fn is_admin_user(ctx: &AppContext, user_id: i64) -> bool {
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

fn admin_home_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "📋 Tất cả lệnh admin",
            ADMIN_MENU_ALL,
        )],
        vec![
            InlineKeyboardButton::callback("📣 Broadcast", ADMIN_MENU_BROADCAST),
            InlineKeyboardButton::callback("🤝 CTV", ADMIN_MENU_AFFILIATE),
        ],
        vec![InlineKeyboardButton::callback("🛒 Đăng sản phẩm", ADMIN_MENU_GROUP)],
        vec![InlineKeyboardButton::callback(
            "💸 Hoàn tiền đơn hàng",
            ADMIN_MENU_REFUND,
        )],
    ])
}

fn admin_back_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "⬅️ Menu admin",
        ADMIN_MENU_HOME,
    )]])
}

fn admin_home_text() -> &'static str {
    "🔐 <b>MENU ADMIN</b>\n\nMình tìm thấy <b>4 lệnh admin</b> trong source, cộng thêm menu mới <code>/admin</code>.\n\nChọn nhóm chức năng bên dưới để xem cách dùng."
}

fn admin_all_commands_text() -> &'static str {
    "📋 <b>TẤT CẢ LỆNH ADMIN</b>\n\n<b>Thông báo</b>\n<code>/broadcast</code> - mở danh sách mẫu thông báo và gửi broadcast.\n\n<b>CTV</b>\n<code>/ctvadd &lt;telegram_id&gt; [hoa_hong_%]</code> - thêm CTV thủ công.\n<code>/ctvoff &lt;telegram_id&gt;</code> - tắt CTV.\n<code>/ctvlist</code> - xem danh sách CTV.\n\n<b>Nhóm bán hàng</b>\n<code>/postproduct &lt;product_id&gt;</code> - đăng card sản phẩm vào group/chat hiện tại.\n\n<b>Hoàn tiền</b>\nKhông có lệnh gõ riêng. Admin bấm nút hoàn tiền trong thông báo đơn hàng."
}

fn admin_broadcast_text() -> &'static str {
    "📣 <b>BROADCAST</b>\n\nLệnh:\n<code>/broadcast</code>\n\nBot sẽ hiện danh sách mẫu thông báo. Admin chọn mẫu, hệ thống đưa vào hàng gửi cho user.\n\nLưu ý: lệnh này hiện chỉ nhận quyền từ <code>TELEGRAM_ICON_ADMIN_IDS</code>."
}

fn admin_affiliate_text() -> &'static str {
    "🤝 <b>QUẢN LÝ CTV</b>\n\n<code>/ctvadd &lt;telegram_id&gt; [hoa_hong_%]</code>\nThêm CTV thủ công. Ví dụ: <code>/ctvadd 123456789 5</code>\n\n<code>/ctvoff &lt;telegram_id&gt;</code>\nTắt CTV. Ví dụ: <code>/ctvoff 123456789</code>\n\n<code>/ctvlist</code>\nXem danh sách CTV, số đơn và hoa hồng.\n\nGhi chú: khách tự đăng ký CTV bằng nút <b>Đăng kí CTV</b> ở <code>/start</code>."
}

fn admin_group_text() -> &'static str {
    "🛒 <b>ĐĂNG SẢN PHẨM VÀO GROUP</b>\n\n<code>/postproduct &lt;product_id&gt;</code>\nĐăng card sản phẩm vào group/chat hiện tại.\n\nVí dụ:\n<code>/postproduct 26</code>\n\nLệnh khách dùng trong nhóm:\n<code>/gshop</code> - mở shop trong bot riêng. Lệnh này không phải admin."
}

fn admin_refund_text() -> &'static str {
    "💸 <b>HOÀN TIỀN ĐƠN HÀNG</b>\n\nKhông có lệnh gõ riêng. Logic hoàn tiền nằm ở nút admin trong thông báo đơn hàng.\n\nCách dùng:\n1. Khi có đơn thanh toán, bot gửi thông báo cho admin.\n2. Admin bấm nút hoàn tiền.\n3. Bot yêu cầu xác nhận.\n4. Khi xác nhận, tiền được cộng lại vào ví user.\n\nLưu ý: quyền hoàn tiền hiện dùng danh sách <code>ORDER_NOTIFICATION_ADMIN_IDS</code>."
}
