mod app;
mod artifact_signature;
mod bot;
mod config;
mod core;
mod db;
mod domains;

use crate::bot::texts::BotTexts;
use anyhow::{Context, Result};
use config::Config;
use std::path::Path;
use teloxide::payloads::SetMyCommandsSetters;
use teloxide::requests::Requester;
use teloxide::types::BotCommand;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    artifact_signature::keep_marker();
    if run_cli_command()? {
        return Ok(());
    }
    init_tracing();

    let config = Config::from_env()?;
    let pool = db::init_pool(&config.database_url).await?;
    let bot = teloxide::Bot::new(config.telegram_token.clone());
    let plugins = crate::bot::plugins::init_plugins();

    // Seed DB with .env config values (only if not already set)
    seed_configs_from_env(&pool, &config).await;

    // Load operational configs from DB and bot text from JSON files.
    let configs = domains::configs::repo::get_all_configs(&pool).await?;
    let texts = domains::i18n::repo::load_texts_from_dir(&config.i18n_dir)?;
    let ctx = app::AppContext::new(
        bot.clone(),
        pool.clone(),
        config.clone(),
        configs,
        texts,
        plugins,
    );
    log_crypto_feature_status(&ctx);

    for plugin in ctx.plugins.iter() {
        if let Err(e) = plugin.on_init(&ctx.pool).await {
            tracing::error!("Plugin {} error on_init: {}", plugin.name(), e);
        }
    }

    let public_commands = public_bot_commands();
    if !public_commands.is_empty() {
        let texts = ctx.texts.read().map(|texts| texts.clone());
        let result = match texts {
            Ok(texts) => register_bot_commands(&bot, &texts, &public_commands).await,
            Err(_) => bot
                .set_my_commands(public_commands)
                .await
                .map(|_| ())
                .map_err(Into::into),
        };

        if let Err(e) = result {
            tracing::error!("Failed to set bot commands: {}", e);
        } else {
            tracing::info!("Successfully registered bot commands to Telegram");
        }
    }

    let bot_task = tokio::spawn({
        let ctx = ctx.clone();
        async move { bot::run(ctx).await }
    });

    let server_task = tokio::spawn({
        let ctx = ctx.clone();
        async move { crate::core::pages::serve(ctx).await }
    });

    let worker_task = tokio::spawn({
        let ctx = ctx.clone();
        async move { domains::worker::run(ctx).await }
    });

    tokio::select! {
        result = wait_for_task("Bot", bot_task) => result,
        result = wait_for_task("Server", server_task) => result,
        result = wait_for_task("Worker", worker_task) => result,
    }
}

async fn wait_for_task(name: &'static str, task: JoinHandle<Result<()>>) -> Result<()> {
    match task.await {
        Ok(Ok(())) => anyhow::bail!("{name} stopped unexpectedly"),
        Ok(Err(err)) => Err(err).with_context(|| format!("{name} failed")),
        Err(err) => Err(err).with_context(|| format!("{name} task panicked or was cancelled")),
    }
}

fn run_cli_command() -> Result<bool> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 {
        return Ok(false);
    }

    match args[1].as_str() {
        "merge-i18n" => {
            if args.len() != 4 {
                anyhow::bail!(
                    "usage: {} merge-i18n SOURCE_I18N_DIR TARGET_I18N_DIR",
                    args[0]
                );
            }
            domains::i18n::repo::merge_i18n_dirs(Path::new(&args[2]), Path::new(&args[3]))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn public_bot_commands() -> Vec<BotCommand> {
    vec![
        BotCommand {
            command: "start".to_string(),
            description: "Bắt đầu".to_string(),
        },
        BotCommand {
            command: "shop".to_string(),
            description: "Xem sản phẩm".to_string(),
        },
        BotCommand {
            command: "wallet".to_string(),
            description: "Xem ví tiền".to_string(),
        },
        BotCommand {
            command: "orders".to_string(),
            description: "Đơn hàng gần đây".to_string(),
        },
        BotCommand {
            command: "order".to_string(),
            description: "Tra cứu đơn theo mã".to_string(),
        },
        BotCommand {
            command: "viameta".to_string(),
            description: "Dịch vụ tích xanh".to_string(),
        },
        BotCommand {
            command: "help".to_string(),
            description: "Hướng dẫn".to_string(),
        },
    ]
}

fn localized_commands_for_lang(
    commands: &[BotCommand],
    texts: &BotTexts,
    lang: &str,
) -> Vec<BotCommand> {
    commands
        .iter()
        .map(|command| {
            let key = format!("cmd_{}", command.command.replace('-', "_"));
            BotCommand {
                command: command.command.clone(),
                description: sanitize_bot_command_description(
                    &texts.get_lang(&key, lang, &command.description),
                ),
            }
        })
        .collect()
}

fn sanitize_bot_command_description(description: &str) -> String {
    let mut rendered = String::with_capacity(description.len());
    let mut byte_index = 0usize;

    while byte_index < description.len() {
        let remaining = &description[byte_index..];
        if let Some(placeholder_len) = custom_emoji_placeholder_len(remaining) {
            rendered.push('✨');
            byte_index += placeholder_len;
        } else if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }

    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn custom_emoji_placeholder_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('{')?;
    let digits_len = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .map(char::len_utf8)
        .sum::<usize>();
    if digits_len == 0 || !rest[digits_len..].starts_with('}') {
        return None;
    }
    Some(1 + digits_len + 1)
}

async fn register_bot_commands(
    bot: &teloxide::Bot,
    texts: &BotTexts,
    commands: &[BotCommand],
) -> Result<()> {
    let default_lang = texts.default_language();
    bot.set_my_commands(localized_commands_for_lang(commands, texts, &default_lang))
        .await?;

    for language in texts.enabled_languages() {
        if let Err(err) = bot
            .set_my_commands(localized_commands_for_lang(commands, texts, &language.code))
            .language_code(language.code.clone())
            .await
        {
            tracing::warn!(
                "Failed to set bot commands for language {}: {}",
                language.code,
                err
            );
        }
    }

    Ok(())
}

fn init_tracing() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}

async fn seed_configs_from_env(pool: &db::DbPool, config: &config::Config) {
    let mut seeds: Vec<(&str, String)> = vec![
        ("bank_name", config.bank_name.clone()),
        ("bot_maintenance_enabled", "0".to_string()),
        (
            "bot_maintenance_message",
            "Bot dang bao tri, vui long quay lai sau.".to_string(),
        ),
        ("required_channel_enabled", "1".to_string()),
        ("required_channel_id", "@zvwboo".to_string()),
        ("required_channel_url", "https://t.me/zvwboo".to_string()),
    ];

    let optional_seeds: Vec<(&str, &Option<String>)> = vec![
        ("bank_account", &config.bank_account),
        ("bank_account_name", &config.bank_account_name),
        ("base_url", &config.base_url),
        ("usdt_rate_custom_url", &config.crypto.rate_custom_url),
    ];

    for (key, val) in optional_seeds {
        if let Some(v) = val {
            seeds.push((key, v.clone()));
        }
    }
    if let Ok(icon_admin_ids) = std::env::var("TELEGRAM_ICON_ADMIN_IDS") {
        push_optional_seed(
            &mut seeds,
            "telegram_icon_admin_ids",
            Some(icon_admin_ids.as_str()),
        );
    }
    seeds.push(("order_notifications_enabled", "0".to_string()));
    if let Ok(order_admin_ids) = std::env::var("ORDER_NOTIFICATION_ADMIN_IDS") {
        push_optional_seed(
            &mut seeds,
            "order_notification_admin_ids",
            Some(order_admin_ids.as_str()),
        );
    }
    if let Ok(viameta_username) = std::env::var("VIAMETA_USERNAME") {
        push_optional_seed(&mut seeds, "viameta_username", Some(viameta_username.as_str()));
    }
    if let Ok(viameta_password) = std::env::var("VIAMETA_PASSWORD") {
        push_optional_seed(&mut seeds, "viameta_password", Some(viameta_password.as_str()));
    }

    push_optional_seed(
        &mut seeds,
        "bep20_merchant_wallet",
        config.crypto.bep20.merchant_wallet.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "bscscan_api_key",
        config.crypto.bep20.bscscan_api_key.as_deref(),
    );
    if let Some(start_block) = config.crypto.bep20.start_block {
        seeds.push(("bep20_start_block", start_block.to_string()));
    }
    push_optional_seed(
        &mut seeds,
        "binance_pay_api_key",
        config.crypto.binance.api_key.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_secret",
        config.crypto.binance.secret.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_api_secret",
        config.crypto.binance.api_secret.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_cert_sn",
        config.crypto.binance.cert_sn.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_receiver_pay_id",
        config.crypto.binance.receiver_pay_id.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_receiver_name",
        config.crypto.binance.receiver_name.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_webhook_url",
        config.crypto.binance.webhook_url.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_return_url",
        config.crypto.binance.return_url.as_deref(),
    );
    push_optional_seed(
        &mut seeds,
        "binance_pay_cancel_url",
        config.crypto.binance.cancel_url.as_deref(),
    );

    seeds.extend([
        (
            "binance_pay_note_enabled",
            if config.crypto.binance.note_enabled {
                "1".to_string()
            } else {
                "0".to_string()
            },
        ),
        (
            "binance_pay_poll_interval_seconds",
            config.crypto.binance.poll_interval_seconds.to_string(),
        ),
        (
            "binance_pay_history_lookback_minutes",
            config.crypto.binance.history_lookback_minutes.to_string(),
        ),
        (
            "binance_pay_recv_window_ms",
            config.crypto.binance.recv_window_ms.to_string(),
        ),
        (
            "binance_pay_match_grace_minutes",
            config.crypto.binance.match_grace_minutes.to_string(),
        ),
        (
            "binance_pay_note_prefix",
            config.crypto.binance.note_prefix.clone(),
        ),
        (
            "binance_pay_note_digits",
            config.crypto.binance.note_digits.to_string(),
        ),
        (
            "binance_pay_amount_tolerance_usdt",
            config.crypto.binance.amount_tolerance_usdt.to_string(),
        ),
        (
            "crypto_pay_ttl_minutes",
            config.crypto.pay_ttl_minutes.to_string(),
        ),
        (
            "usdt_rate_buffer_percent",
            config.crypto.usdt_rate_buffer_percent.to_string(),
        ),
        (
            "usdt_rate_cache_seconds",
            config.crypto.usdt_rate_cache_seconds.to_string(),
        ),
        (
            "usdt_rate_stale_seconds",
            config.crypto.usdt_rate_stale_seconds.to_string(),
        ),
        (
            "usd_vnd_fallback_rate",
            config.crypto.usd_vnd_fallback_rate.to_string(),
        ),
        (
            "bep20_usdt_contract",
            config.crypto.bep20.usdt_contract.clone(),
        ),
        (
            "bep20_required_confirmations",
            config.crypto.bep20.required_confirmations.to_string(),
        ),
    ]);

    for (key, value) in seeds {
        if let Err(e) = sqlx::query("INSERT OR IGNORE INTO app_configs (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool)
            .await
        {
            tracing::warn!("Failed to seed config {key}: {e}");
        }
    }

    cleanup_legacy_i18n_configs(pool).await;
    tracing::info!("Operational config values seeded from .env and defaults to DB");
}

fn push_optional_seed(seeds: &mut Vec<(&str, String)>, key: &'static str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    seeds.push((key, value.to_string()));
}

async fn cleanup_legacy_i18n_configs(pool: &db::DbPool) {
    if let Err(e) = sqlx::query(
        r#"
        DELETE FROM app_configs
        WHERE key = 'i18n_languages'
           OR key LIKE '%_vi'
           OR key LIKE '%_en'
           OR key IN ('start', 'help', 'start_btn_shop', 'start_btn_wallet', 'start_btn_help', 'viameta_api_key')
        "#,
    )
    .execute(pool)
    .await
    {
        tracing::warn!("Failed to clean legacy i18n config keys: {e}");
    }
}

fn log_crypto_feature_status(ctx: &std::sync::Arc<app::AppContext>) {
    if ctx.usdt_payments_enabled() {
        tracing::info!("USDT payments: ENABLED");
    } else {
        tracing::info!("USDT payments: DISABLED");
    }

    if ctx.binance_pay_enabled() {
        let env = match ctx.config.crypto.binance.env {
            config::BinancePayEnv::Sandbox => "sandbox",
            config::BinancePayEnv::Production => "production",
        };
        tracing::info!("Binance Pay: ENABLED ({env})");
    } else {
        let reason = ctx
            .binance_pay_disabled_reason()
            .unwrap_or_else(|| "not configured".to_string());
        tracing::info!("Binance Pay: DISABLED ({reason})");
    }

    if ctx.bep20_enabled() {
        let wallet = ctx.bep20_merchant_wallet().unwrap_or_default();
        let short_wallet = if wallet.len() >= 10 {
            format!("{}...{}", &wallet[..6], &wallet[wallet.len() - 4..])
        } else {
            wallet.to_string()
        };
        tracing::info!(
            "BEP20 USDT: ENABLED (wallet: {}, confirmations: {})",
            short_wallet,
            ctx.bep20_required_confirmations()
        );
    } else {
        let reason = ctx
            .bep20_disabled_reason()
            .unwrap_or_else(|| "not configured".to_string());
        tracing::info!("BEP20 USDT: DISABLED ({reason})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::{BotTexts, LanguageInfo};
    use std::collections::HashMap;
    use teloxide::types::BotCommand;

    fn test_config() -> config::Config {
        config::Config {
            telegram_token: "test-token".to_string(),
            database_url: ":memory:".to_string(),
            bank_name: "VCB".to_string(),
            bank_account: None,
            bank_account_name: None,
            webhook_secret: "test-webhook-secret".to_string(),
            admin_jwt_secret: "test-admin-jwt-secret-at-least-32-chars".to_string(),
            admin_setup_code: "setup-code".to_string(),
            admin_cookie_secure: false,
            base_url: None,
            i18n_dir: "i18n".to_string(),
            port: 8080,
            crypto: config::CryptoConfig::default(),
        }
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            r#"
            CREATE TABLE app_configs (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn config_value(pool: &sqlx::SqlitePool, key: &str) -> Option<String> {
        use sqlx::Row;

        sqlx::query("SELECT value FROM app_configs WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .unwrap()
            .map(|row| row.get("value"))
    }

    #[tokio::test]
    async fn seeding_keeps_i18n_out_of_app_configs_and_cleans_legacy_keys() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO app_configs (key, value) VALUES (?, ?), (?, ?), (?, ?), (?, ?)")
            .bind("language_btn_vi")
            .bind("legacy vi")
            .bind("start_en")
            .bind("legacy start")
            .bind("i18n_languages")
            .bind("[]")
            .bind("required_channel_url")
            .bind("https://t.me/custom")
            .execute(&pool)
            .await
            .unwrap();

        seed_configs_from_env(&pool, &test_config()).await;

        assert_eq!(config_value(&pool, "language_btn_vi").await, None);
        assert_eq!(config_value(&pool, "start_en").await, None);
        assert_eq!(config_value(&pool, "i18n_languages").await, None);
        assert_eq!(
            config_value(&pool, "required_channel_url").await,
            Some("https://t.me/custom".to_string())
        );
        assert_eq!(
            config_value(&pool, "bank_name").await,
            Some("VCB".to_string())
        );
        assert_eq!(config_value(&pool, "binance_pay_api_key").await, None);
        assert_eq!(config_value(&pool, "binance_pay_secret").await, None);
        assert_eq!(config_value(&pool, "binance_pay_cert_sn").await, None);
        assert_eq!(config_value(&pool, "bep20_merchant_wallet").await, None);
        assert_eq!(config_value(&pool, "bscscan_api_key").await, None);
    }

    #[test]
    fn env_example_does_not_expose_admin_only_i18n_emoji_toggle() {
        const ENV_EXAMPLE: &str = include_str!("../.env.example");
        const ADMIN_ONLY_TOGGLE_ENV: &str = concat!("TELEGRAM_I18N_", "EMOJIS_ENABLED");

        assert!(!ENV_EXAMPLE.contains(ADMIN_ONLY_TOGGLE_ENV));
    }

    #[test]
    fn deploy_scripts_copy_i18n_files() {
        const DEPLOY_SH: &str = include_str!("../deploy.sh");
        const BOT_CLONE_SH: &str = include_str!("../bot_clone.sh");
        const BOT_UPDATE_SH: &str = include_str!("../bot_update.sh");

        assert!(DEPLOY_SH.contains("i18n"));
        assert!(BOT_CLONE_SH.contains("i18n"));
        assert!(BOT_UPDATE_SH.contains("i18n"));
    }

    #[test]
    fn public_bot_commands_hide_admin_only_commands() {
        let commands = public_bot_commands()
            .into_iter()
            .map(|command| command.command)
            .collect::<Vec<_>>();

        assert!(commands.contains(&"start".to_string()));
        assert!(commands.contains(&"shop".to_string()));
        assert!(commands.contains(&"wallet".to_string()));
        assert!(!commands.contains(&"newapi".to_string()));
        assert!(!commands.contains(&"tut".to_string()));
        assert!(!commands.contains(&"myvip".to_string()));
        assert!(!commands.contains(&"ctvlist".to_string()));
        assert!(!commands.contains(&"childbotadd".to_string()));
        assert!(!commands.contains(&"tutadd".to_string()));
    }

    #[test]
    fn localized_commands_use_i18n_descriptions_by_language() {
        let texts = BotTexts::from_language_maps(
            vec![
                LanguageInfo {
                    code: "en".to_string(),
                    label: "English".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
                LanguageInfo {
                    code: "vi".to_string(),
                    label: "Tiếng Việt".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
            ],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    ("cmd_start".to_string(), "Bắt đầu".to_string()),
                    ("cmd_shop".to_string(), "Xem sản phẩm".to_string()),
                ]),
            )]),
        );
        let base = vec![
            BotCommand {
                command: "start".to_string(),
                description: "Start".to_string(),
            },
            BotCommand {
                command: "shop".to_string(),
                description: "View products".to_string(),
            },
        ];

        let commands = localized_commands_for_lang(&base, &texts, "vi");

        assert_eq!(commands[0].description, "Bắt đầu");
        assert_eq!(commands[1].description, "Xem sản phẩm");
    }

    #[test]
    fn localized_commands_render_custom_emoji_placeholders_as_fallback() {
        let texts = BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([(
                    "cmd_help".to_string(),
                    "{5253742260054409879} Hỗ trợ".to_string(),
                )]),
            )]),
        );
        let base = vec![BotCommand {
            command: "help".to_string(),
            description: "Help".to_string(),
        }];

        let commands = localized_commands_for_lang(&base, &texts, "vi");

        assert_eq!(commands[0].description, "✨ Hỗ trợ");
    }

    #[tokio::test]
    async fn completed_background_task_is_reported_as_an_error() {
        let task = tokio::spawn(async { Ok(()) });

        let err = wait_for_task("Bot", task).await.unwrap_err();

        assert!(err.to_string().contains("Bot stopped unexpectedly"));
    }
}
