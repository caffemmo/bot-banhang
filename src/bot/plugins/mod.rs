pub mod cmd_admin_menu;
pub mod cmd_affiliate;
pub mod cmd_api;
pub mod cmd_broadcast;
pub mod cmd_childbot;
pub mod cmd_facebook_unlock;
pub mod cmd_group_sales;
pub mod cmd_help;
pub mod cmd_orders;
pub mod cmd_sale_hunt;
pub mod cmd_shop;
pub mod cmd_start;
pub mod cmd_start_affiliate;
pub mod cmd_tut;
pub mod cmd_tut_delete;
pub mod cmd_tut_public;
pub mod cmd_viameta;
pub mod cmd_wallet;
pub mod example;

use crate::app::AppContext;
use crate::bot::BotDialogue;
use std::sync::Arc;
use teloxide::types::{BotCommand, CallbackQuery, Message};

#[async_trait::async_trait]
pub trait AppPlugin: Send + Sync {
    fn name(&self) -> &'static str;

    async fn on_init(&self, _pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![]
    }

    async fn handle_message(
        &self,
        _ctx: Arc<AppContext>,
        _msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        Ok(false)
    }

    async fn handle_callback(
        &self,
        _ctx: Arc<AppContext>,
        _q: CallbackQuery,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        Ok(false)
    }

    async fn on_order_paid(
        &self,
        _ctx: Arc<AppContext>,
        _order: &crate::domains::orders::models::Order,
        _product: &crate::domains::products::models::Product,
    ) -> Result<Option<String>, anyhow::Error> {
        Ok(None)
    }
}

pub fn init_plugins() -> Vec<Box<dyn AppPlugin>> {
    vec![
        Box::new(cmd_start_affiliate::StartAffiliatePlugin),
        Box::new(cmd_start::StartCommandPlugin),
        Box::new(cmd_admin_menu::AdminMenuCommandPlugin),
        Box::new(cmd_affiliate::AffiliateCommandPlugin),
        Box::new(cmd_childbot::ChildBotCommandPlugin),
        Box::new(cmd_facebook_unlock::FacebookUnlockCommandPlugin),
        Box::new(cmd_tut_public::TutPublicCommandPlugin),
        Box::new(cmd_tut_delete::TutDeleteCommandPlugin),
        Box::new(cmd_tut::TutCommandPlugin),
        Box::new(cmd_help::HelpCommandPlugin),
        Box::new(cmd_api::ApiCommandPlugin),
        Box::new(cmd_broadcast::BroadcastCommandPlugin),
        Box::new(cmd_sale_hunt::SaleHuntCommandPlugin),
        Box::new(cmd_shop::ShopCommandPlugin),
        Box::new(cmd_viameta::ViametaCommandPlugin),
        Box::new(cmd_group_sales::GroupSalesCommandPlugin),
        Box::new(cmd_orders::OrdersCommandPlugin),
        Box::new(cmd_wallet::WalletCommandPlugin),
        Box::new(example::ExamplePlugin),
    ]
}
