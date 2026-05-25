#![allow(dead_code)]

use axum::{Router, middleware};
use std::sync::Arc;

use crate::app::AppContext;

pub mod auth;
pub mod chat;
pub mod client;
pub mod configs;
pub mod crypto_pay;
pub mod i18n;
pub mod orders;
pub mod products;
pub mod stats;
pub mod users;
pub mod wallet;
pub mod worker;

pub fn router(ctx: Arc<AppContext>) -> Router<Arc<AppContext>> {
    let admin_routes = Router::new()
        .merge(auth::api::admin_router())
        .merge(chat::api::router())
        .merge(configs::api::router())
        .merge(crypto_pay::api::router())
        .merge(i18n::api::router())
        .merge(products::api::router())
        .merge(orders::api::router())
        .merge(orders::webhook_events::router())
        .merge(stats::api::router())
        .merge(users::broadcast::router())
        .merge(wallet::api::router())
        .route_layer(middleware::from_fn_with_state(
            ctx,
            auth::api::require_admin_session,
        ));

    Router::new()
        .merge(auth::api::router())
        .merge(client::api::router())
        .merge(orders::webhook::router())
        .merge(admin_routes)
}
