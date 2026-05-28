use crate::app::AppContext;
use crate::domains;
use anyhow::Result;
use axum::routing::get;
use axum::{Router, extract::DefaultBodyLimit};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

pub async fn serve(ctx: Arc<AppContext>) -> Result<()> {
    const MAX_REQUEST_BODY_BYTES: usize = 20 * 1024 * 1024; // 20MB

    let port = ctx.config.port;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Web server listening on {}", addr);

    let app = Router::new()
        .nest_service(
            "/uploads",
            ServeDir::new("storage/uploads").fallback(ServeDir::new("public/uploads")),
        )
        .route_service("/admin", ServeFile::new("public/admin.html"))
        .route_service("/chat.html", ServeFile::new("public/chat.html"))
        .route_service("/chat", ServeFile::new("public/chat.html"))
        .fallback_service(ServeDir::new("public"))
        .route("/ping", get(|| async { "pong" }))
        .merge(crate::core::health::router())
        .merge(domains::router(ctx.clone()))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(ctx.clone());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    const ADMIN_HTML: &str = include_str!("../../public/admin.html");
    const ADMIN_CORE_JS: &str = include_str!("../../public/assets/admin/core.js");
    const ADMIN_EVENTS_JS: &str = include_str!("../../public/assets/admin/events.js");
    const ADMIN_PRODUCTS_JS: &str = include_str!("../../public/assets/admin/products.js");
    const ADMIN_CONFIGS_JS: &str = include_str!("../../public/assets/admin/configs.js");
    const ADMIN_I18N_JS: &str = include_str!("../../public/assets/admin/i18n.js");

    #[test]
    fn admin_i18n_emoji_editor_is_hidden_until_enabled() {
        assert!(!ADMIN_I18N_JS.contains("Gắn emoji trước text này"));
        assert!(!ADMIN_I18N_JS.contains("bot-i18n-emojis-toggle"));
        assert!(ADMIN_HTML.contains("/assets/admin/events.js?v="));
    }

    #[test]
    fn admin_exposes_global_i18n_emoji_enable_switch() {
        assert!(ADMIN_CONFIGS_JS.contains("telegram_i18n_emojis_enabled"));
        assert!(ADMIN_CONFIGS_JS.contains("Bật emoji trước text i18n"));
        assert!(ADMIN_I18N_JS.contains("detail.emojis_enabled === true"));
        assert!(ADMIN_I18N_JS.contains("{Custom emoji ID}"));
        assert!(!ADMIN_I18N_JS.contains("bot-i18n-prefix-emojis"));
        assert!(!ADMIN_I18N_JS.contains("Emoji trước text đang tắt"));
        assert!(!ADMIN_I18N_JS.contains("Emoji động trong nội dung text"));
    }

    #[test]
    fn admin_i18n_editor_exposes_keyboard_button_custom_emoji_fields() {
        assert!(ADMIN_I18N_JS.contains("isKeyboardButtonI18nKey"));
        assert!(ADMIN_I18N_JS.contains("bot-i18n-emoji-fallback"));
        assert!(ADMIN_I18N_JS.contains("bot-i18n-emoji-custom-id"));
        assert!(ADMIN_I18N_JS.contains("Emoji thường"));
        assert!(ADMIN_I18N_JS.contains("Custom emoji ID nút"));
        assert!(ADMIN_I18N_JS.contains("payload.emojis = buildBotI18nEmojiPayload()"));
        assert!(ADMIN_I18N_JS.contains("start_btn_shop"));
    }

    #[test]
    fn admin_broadcast_uses_inline_custom_emoji_placeholders() {
        assert!(!ADMIN_HTML.contains("id=\"bc-emoji-prefix\""));
        assert!(!ADMIN_HTML.contains("Emoji mở đầu"));
        assert!(!ADMIN_HTML.contains("id=\"bc-custom-emojis\""));
        assert!(!ADMIN_HTML.contains("Emoji động trong câu thông báo"));
        assert!(!ADMIN_CORE_JS.contains("custom_emojis"));
        assert!(!ADMIN_CORE_JS.contains("formData.append('emoji_prefix'"));
        assert!(!ADMIN_CORE_JS.contains("formData.append('custom_emojis'"));
        assert!(!ADMIN_HTML.contains("bc-sticker-ids"));
        assert!(!ADMIN_CORE_JS.contains("sticker_ids"));
    }

    #[test]
    fn admin_product_modal_exposes_category_and_button_emoji() {
        assert!(ADMIN_HTML.contains("id=\"product-categories-body\""));
        assert!(ADMIN_HTML.contains("id=\"new-product-category\""));
        assert!(ADMIN_HTML.contains("id=\"product-category-id\""));
        assert!(ADMIN_PRODUCTS_JS.contains("/product-categories"));
        assert!(ADMIN_PRODUCTS_JS.contains("category_id"));
        assert!(ADMIN_HTML.contains("id=\"product-button-emoji\""));
        assert!(ADMIN_HTML.contains("id=\"product-button-custom-emoji-id\""));
        assert!(ADMIN_HTML.contains("Emoji nút"));
        assert!(ADMIN_HTML.contains("Custom emoji ID động"));
    }

    #[test]
    fn admin_product_modal_exposes_sold_count_toggle() {
        assert!(ADMIN_HTML.contains("id=\"product-show-sold-count\""));
        assert!(ADMIN_HTML.contains("Hiển thị số đã bán"));
        assert!(ADMIN_PRODUCTS_JS.contains("show_sold_count"));
    }

    #[test]
    fn admin_broadcast_templates_are_scoped_to_selected_mode() {
        assert!(ADMIN_CORE_JS.contains("filteredBroadcastTemplates()"));
        assert!(ADMIN_CORE_JS.contains("renderBroadcastTemplateOptions({ preferredId"));
        assert!(ADMIN_CORE_JS.contains("Không có mẫu cho kiểu"));
        assert!(ADMIN_EVENTS_JS.contains("renderBroadcastTemplateOptions()"));
        assert!(ADMIN_EVENTS_JS.contains("selectedBroadcastTemplate()"));
        assert!(ADMIN_EVENTS_JS.contains("applyBroadcastTemplate(template.id)"));
        assert!(ADMIN_EVENTS_JS.contains("toggleBroadcastProductPicker()"));
    }

    #[test]
    fn admin_broadcast_documents_all_supported_callback_patterns() {
        assert!(ADMIN_HTML.contains("Callback hỗ trợ đầy đủ"));
        assert!(ADMIN_HTML.contains("start:menu"));
        assert!(ADMIN_HTML.contains("start:shop"));
        assert!(ADMIN_HTML.contains("start:wallet"));
        assert!(ADMIN_HTML.contains("start:orders"));
        assert!(ADMIN_HTML.contains("start:help"));
        assert!(ADMIN_HTML.contains("start:language"));
        assert!(ADMIN_HTML.contains("wallet:topup"));
        assert!(ADMIN_HTML.contains("wallet:topup_usdt"));
        assert!(ADMIN_HTML.contains("wallet:topup_binance"));
        assert!(ADMIN_HTML.contains("wallet:topup_history"));
        assert!(ADMIN_HTML.contains("wallet:show"));
        assert!(ADMIN_HTML.contains("buy:ID"));
        assert!(ADMIN_HTML.contains("shop_api"));
        assert!(ADMIN_HTML.contains("shop_api_new"));
    }
}
