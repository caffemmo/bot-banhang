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
}
