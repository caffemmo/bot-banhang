use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bot::plugins::AppPlugin;
use crate::bot::texts::BotTexts;
use crate::config::is_eth_address;
use crate::core::exchange_rate::RateCache;
use crate::{config::Config, db::DbPool};
use teloxide::prelude::*;
use tokio::sync::RwLock as TokioRwLock;

pub const ORDER_MEMO_PREFIX_DEFAULT: &str = "PTN1411";
pub const ORDER_MEMO_LENGTH_DEFAULT: usize = 10;
pub const ORDER_MEMO_LENGTH_MIN: usize = 10;
pub const ORDER_MEMO_LENGTH_MAX: usize = 16;
pub const ORDER_MEMO_PREFIX_MAX_LEN: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct I18nEmoji {
    pub fallback: String,
    #[serde(default)]
    pub custom_emoji_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct I18nEmojiPrefix {
    pub fallback: String,
    #[serde(default)]
    pub custom_emoji_id: Option<String>,
    #[serde(default)]
    pub emojis: Vec<I18nEmoji>,
}

impl I18nEmojiPrefix {
    pub fn from_json_value(value: &Value) -> Option<Self> {
        i18n_emoji_prefix_from_value(value)
    }
}

#[derive(Clone)]
pub struct AppContext {
    pub bot: Bot,
    pub pool: DbPool,
    pub config: Config,
    pub configs: Arc<RwLock<HashMap<String, String>>>,
    pub texts: Arc<RwLock<BotTexts>>,
    pub plugins: Arc<Vec<Box<dyn AppPlugin>>>,
    #[allow(dead_code)]
    pub usdt_rate_cache: Arc<TokioRwLock<Option<RateCache>>>,
}

impl AppContext {
    pub fn new(
        bot: Bot,
        pool: DbPool,
        config: Config,
        configs: HashMap<String, String>,
        texts: BotTexts,
        plugins: Vec<Box<dyn AppPlugin>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bot,
            pool,
            config,
            configs: Arc::new(RwLock::new(configs)),
            texts: Arc::new(RwLock::new(texts)),
            plugins: Arc::new(plugins),
            usdt_rate_cache: Arc::new(TokioRwLock::new(None)),
        })
    }

    /// Get an operational config value by key with a default fallback.
    pub fn get_text(&self, key: &str, default: &str) -> String {
        self.get_config_value(key, default)
    }

    pub fn get_text_lang(&self, key: &str, lang: &str, default: &str) -> String {
        self.texts.read().unwrap().get_lang(key, lang, default)
    }

    pub fn normalize_language_code(&self, language_code: Option<&str>) -> String {
        self.texts.read().unwrap().normalize_language(language_code)
    }

    pub fn is_supported_language(&self, language_code: &str) -> bool {
        self.texts
            .read()
            .unwrap()
            .is_supported_language(language_code)
    }

    pub fn render_text_lang(
        &self,
        key: &str,
        lang: &str,
        default: &str,
        vars: &[(&str, String)],
    ) -> String {
        self.texts
            .read()
            .unwrap()
            .render_lang(key, lang, default, vars)
    }

    /// Update bot texts (for future admin API)
    pub fn update_texts(&self, new_texts: BotTexts) {
        if let Ok(mut texts) = self.texts.write() {
            *texts = new_texts;
        }
    }

    pub fn update_configs(&self, new_configs: HashMap<String, String>) {
        if let Ok(mut configs) = self.configs.write() {
            *configs = new_configs;
        }
    }

    fn get_config_value(&self, key: &str, default: &str) -> String {
        self.configs
            .read()
            .ok()
            .map(|configs| config_value_from_map(&configs, key, default))
            .unwrap_or_else(|| default.to_string())
    }

    fn get_config_value_opt(&self, key: &str, fallback: Option<&str>) -> Option<String> {
        let val = self.get_config_value(key, fallback.unwrap_or(""));
        if val.trim().is_empty() {
            None
        } else {
            Some(val)
        }
    }

    // ---- DB-backed config helpers (fallback to .env) ----

    pub fn bank_name(&self) -> String {
        self.get_config_value("bank_name", &self.config.bank_name)
    }

    pub fn bank_account(&self) -> String {
        let fallback = self.config.bank_account.as_deref().unwrap_or("");
        self.get_config_value("bank_account", fallback)
    }

    pub fn bank_account_name(&self) -> Option<String> {
        let val = self.get_config_value(
            "bank_account_name",
            self.config.bank_account_name.as_deref().unwrap_or(""),
        );
        if val.is_empty() { None } else { Some(val) }
    }

    #[allow(dead_code)]
    pub fn base_url(&self) -> Option<String> {
        let runtime_base_url = self
            .configs
            .read()
            .ok()
            .and_then(|configs| configs.get("base_url").cloned())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        runtime_base_url.or_else(|| {
            self.config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    }

    pub fn binance_pay_enabled(&self) -> bool {
        self.binance_pay_note_enabled()
            && self.binance_pay_api_key().is_some()
            && self.binance_pay_api_secret().is_some()
            && self.binance_pay_receiver_pay_id().is_some()
            && self.binance_pay_receiver_name().is_some()
    }

    pub fn binance_pay_disabled_reason(&self) -> Option<String> {
        if self.binance_pay_enabled() {
            return None;
        }
        let mut missing = Vec::new();
        if self.binance_pay_api_key().is_none() {
            missing.push("binance_pay_api_key");
        }
        if self.binance_pay_api_secret().is_none() {
            missing.push("binance_pay_api_secret");
        }
        if self.binance_pay_receiver_pay_id().is_none() {
            missing.push("binance_pay_receiver_pay_id");
        }
        if self.binance_pay_receiver_name().is_none() {
            missing.push("binance_pay_receiver_name");
        }
        Some(format!("missing admin config: {}", missing.join("/")))
    }

    pub fn bep20_enabled(&self) -> bool {
        self.bep20_merchant_wallet()
            .as_deref()
            .is_some_and(is_eth_address)
            && self.bep20_bscscan_api_key().is_some()
    }

    pub fn bep20_disabled_reason(&self) -> Option<String> {
        if self.bep20_enabled() {
            return None;
        }
        let mut missing = Vec::new();
        match self.bep20_merchant_wallet() {
            Some(wallet) if !is_eth_address(&wallet) => {
                return Some("invalid admin config: bep20_merchant_wallet".to_string());
            }
            Some(_) => {}
            None => missing.push("bep20_merchant_wallet"),
        }
        if self.bep20_bscscan_api_key().is_none() {
            missing.push("bscscan_api_key");
        }
        Some(format!("missing admin config: {}", missing.join("/")))
    }

    pub fn usdt_payments_enabled(&self) -> bool {
        self.binance_pay_enabled() || self.bep20_enabled()
    }

    pub fn is_telegram_icon_admin(&self, user_id: i64) -> bool {
        self.telegram_icon_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
    }

    pub fn telegram_icon_admin_ids(&self) -> Vec<i64> {
        self.get_config_value("telegram_icon_admin_ids", "")
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter_map(|raw| raw.trim().parse::<i64>().ok())
            .collect()
    }

    #[allow(dead_code)]
    pub fn i18n_emoji_for_key(&self, key: &str) -> Option<String> {
        self.i18n_emoji_prefix_for_key(key)
            .map(|prefix| prefix.fallback)
    }

    pub fn i18n_emoji_prefix_for_key(&self, key: &str) -> Option<I18nEmojiPrefix> {
        if !self.i18n_emojis_enabled() {
            return None;
        }
        let raw = self.get_config_value("telegram_i18n_emojis", "");
        let Ok(map) = serde_json::from_str::<HashMap<String, Value>>(&raw) else {
            return None;
        };
        map.get(key).and_then(i18n_emoji_prefix_from_value)
    }

    pub fn i18n_emojis_enabled(&self) -> bool {
        matches!(
            self.get_config_value("telegram_i18n_emojis_enabled", "0")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    pub fn custom_emoji_map(&self) -> HashMap<String, String> {
        if !self.i18n_emojis_enabled() {
            return HashMap::new();
        }
        let raw = self.get_config_value("telegram_custom_emojis", "");
        custom_emoji_map_from_values(
            serde_json::from_str::<HashMap<String, Value>>(&raw).unwrap_or_default(),
        )
    }

    pub fn crypto_pay_ttl_minutes(&self) -> i64 {
        self.get_config_value(
            "crypto_pay_ttl_minutes",
            &self.config.crypto.pay_ttl_minutes.to_string(),
        )
        .parse::<i64>()
        .ok()
        .filter(|v| (1..=1440).contains(v))
        .unwrap_or(self.config.crypto.pay_ttl_minutes)
    }

    pub fn usdt_rate_buffer_percent(&self) -> Decimal {
        self.get_config_value(
            "usdt_rate_buffer_percent",
            &self.config.crypto.usdt_rate_buffer_percent.to_string(),
        )
        .parse::<Decimal>()
        .ok()
        .filter(|v| *v >= Decimal::ZERO && *v <= Decimal::from(10))
        .unwrap_or(self.config.crypto.usdt_rate_buffer_percent)
    }

    pub fn usdt_rate_cache_seconds(&self) -> i64 {
        self.get_config_value(
            "usdt_rate_cache_seconds",
            &self.config.crypto.usdt_rate_cache_seconds.to_string(),
        )
        .parse::<i64>()
        .unwrap_or(self.config.crypto.usdt_rate_cache_seconds as i64)
    }

    pub fn usdt_rate_stale_seconds(&self) -> i64 {
        self.get_config_value(
            "usdt_rate_stale_seconds",
            &self.config.crypto.usdt_rate_stale_seconds.to_string(),
        )
        .parse::<i64>()
        .unwrap_or(self.config.crypto.usdt_rate_stale_seconds as i64)
    }

    pub fn usd_vnd_fallback_rate(&self) -> Decimal {
        self.get_config_value(
            "usd_vnd_fallback_rate",
            &self.config.crypto.usd_vnd_fallback_rate.to_string(),
        )
        .parse::<Decimal>()
        .ok()
        .filter(|v| *v > Decimal::ZERO)
        .unwrap_or(self.config.crypto.usd_vnd_fallback_rate)
    }

    pub fn usdt_rate_custom_url(&self) -> Option<String> {
        self.get_config_value_opt(
            "usdt_rate_custom_url",
            self.config.crypto.rate_custom_url.as_deref(),
        )
    }

    pub fn bep20_merchant_wallet(&self) -> Option<String> {
        self.get_config_value_opt(
            "bep20_merchant_wallet",
            self.config.crypto.bep20.merchant_wallet.as_deref(),
        )
    }

    pub fn bep20_usdt_contract(&self) -> String {
        self.get_config_value(
            "bep20_usdt_contract",
            &self.config.crypto.bep20.usdt_contract,
        )
    }

    pub fn bep20_required_confirmations(&self) -> i64 {
        self.get_config_value(
            "bep20_required_confirmations",
            &self.config.crypto.bep20.required_confirmations.to_string(),
        )
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 1)
        .unwrap_or(self.config.crypto.bep20.required_confirmations)
    }

    pub fn bep20_start_block(&self) -> Option<i64> {
        let fallback = self.config.crypto.bep20.start_block.map(|v| v.to_string());
        self.get_config_value_opt("bep20_start_block", fallback.as_deref())
            .and_then(|v| v.parse::<i64>().ok())
    }

    pub fn bep20_bscscan_api_key(&self) -> Option<String> {
        self.get_config_value_opt(
            "bscscan_api_key",
            self.config.crypto.bep20.bscscan_api_key.as_deref(),
        )
    }

    pub fn binance_pay_api_key(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_api_key",
            self.config.crypto.binance.api_key.as_deref(),
        )
    }

    pub fn binance_pay_secret(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_secret",
            self.config.crypto.binance.secret.as_deref(),
        )
    }

    pub fn binance_pay_api_secret(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_api_secret",
            self.config.crypto.binance.api_secret.as_deref(),
        )
    }

    pub fn binance_pay_cert_sn(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_cert_sn",
            self.config.crypto.binance.cert_sn.as_deref(),
        )
    }

    pub fn binance_pay_webhook_url(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_webhook_url",
            self.config.crypto.binance.webhook_url.as_deref(),
        )
    }

    pub fn binance_pay_return_url(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_return_url",
            self.config.crypto.binance.return_url.as_deref(),
        )
    }

    pub fn binance_pay_cancel_url(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_cancel_url",
            self.config.crypto.binance.cancel_url.as_deref(),
        )
    }

    pub fn binance_pay_note_enabled(&self) -> bool {
        matches!(
            self.get_config_value(
                "binance_pay_note_enabled",
                if self.config.crypto.binance.note_enabled {
                    "1"
                } else {
                    "0"
                },
            )
            .trim()
            .to_ascii_lowercase()
            .as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    pub fn binance_pay_receiver_pay_id(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_receiver_pay_id",
            self.config.crypto.binance.receiver_pay_id.as_deref(),
        )
        .filter(|v| (5..=32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()))
    }

    pub fn binance_pay_receiver_name(&self) -> Option<String> {
        self.get_config_value_opt(
            "binance_pay_receiver_name",
            self.config.crypto.binance.receiver_name.as_deref(),
        )
        .filter(|v| (1..=64).contains(&v.len()))
    }

    pub fn binance_pay_poll_interval_seconds(&self) -> u64 {
        self.get_config_value(
            "binance_pay_poll_interval_seconds",
            &self.config.crypto.binance.poll_interval_seconds.to_string(),
        )
        .parse::<u64>()
        .ok()
        .map(|v| v.clamp(15, 300))
        .unwrap_or(self.config.crypto.binance.poll_interval_seconds)
    }

    pub fn binance_pay_history_lookback_minutes(&self) -> i64 {
        self.get_config_value(
            "binance_pay_history_lookback_minutes",
            &self
                .config
                .crypto
                .binance
                .history_lookback_minutes
                .to_string(),
        )
        .parse::<i64>()
        .ok()
        .map(|v| v.clamp(10, 1440))
        .unwrap_or(self.config.crypto.binance.history_lookback_minutes)
    }

    pub fn binance_pay_recv_window_ms(&self) -> i64 {
        self.get_config_value(
            "binance_pay_recv_window_ms",
            &self.config.crypto.binance.recv_window_ms.to_string(),
        )
        .parse::<i64>()
        .ok()
        .map(|v| v.clamp(1000, 60000))
        .unwrap_or(self.config.crypto.binance.recv_window_ms)
    }

    pub fn binance_pay_match_grace_minutes(&self) -> i64 {
        self.get_config_value(
            "binance_pay_match_grace_minutes",
            &self.config.crypto.binance.match_grace_minutes.to_string(),
        )
        .parse::<i64>()
        .ok()
        .map(|v| v.clamp(0, 1440))
        .unwrap_or(self.config.crypto.binance.match_grace_minutes)
    }

    pub fn binance_pay_note_prefix(&self) -> String {
        let normalized = self
            .get_config_value(
                "binance_pay_note_prefix",
                &self.config.crypto.binance.note_prefix,
            )
            .trim()
            .to_ascii_uppercase()
            .chars()
            .filter(|ch| ch.is_ascii_uppercase())
            .take(8)
            .collect::<String>();
        if normalized.is_empty() {
            self.config.crypto.binance.note_prefix.clone()
        } else {
            normalized
        }
    }

    pub fn binance_pay_note_digits(&self) -> u8 {
        self.get_config_value(
            "binance_pay_note_digits",
            &self.config.crypto.binance.note_digits.to_string(),
        )
        .parse::<u8>()
        .ok()
        .map(|v| v.clamp(4, 12))
        .unwrap_or(self.config.crypto.binance.note_digits)
    }

    pub fn binance_pay_amount_tolerance_usdt(&self) -> Decimal {
        self.get_config_value(
            "binance_pay_amount_tolerance_usdt",
            &self.config.crypto.binance.amount_tolerance_usdt.to_string(),
        )
        .parse::<Decimal>()
        .ok()
        .filter(|v| *v >= Decimal::ZERO)
        .unwrap_or(self.config.crypto.binance.amount_tolerance_usdt)
    }

    pub fn order_memo_prefix(&self) -> String {
        normalize_order_memo_prefix(
            &self.get_config_value("order_memo_prefix", ORDER_MEMO_PREFIX_DEFAULT),
        )
        .unwrap_or_else(|| ORDER_MEMO_PREFIX_DEFAULT.to_string())
    }

    pub fn order_memo_length(&self) -> usize {
        self.get_config_value("order_memo_length", &ORDER_MEMO_LENGTH_DEFAULT.to_string())
            .parse::<usize>()
            .ok()
            .filter(|v| (ORDER_MEMO_LENGTH_MIN..=ORDER_MEMO_LENGTH_MAX).contains(v))
            .unwrap_or(ORDER_MEMO_LENGTH_DEFAULT)
    }
}

fn config_value_from_map(configs: &HashMap<String, String>, key: &str, default: &str) -> String {
    configs
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

pub fn normalize_order_memo_prefix(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_uppercase();
    if value.is_empty()
        || value.len() > ORDER_MEMO_PREFIX_MAX_LEN
        || !value.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(value)
}

fn normalize_i18n_emoji(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > 8
        || value.chars().any(char::is_control)
        || value.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(value.to_string())
}

fn normalize_custom_emoji_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.len() < 8 || value.len() > 64 || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(value.to_string())
}

fn i18n_emoji_prefix_from_value(value: &Value) -> Option<I18nEmojiPrefix> {
    match value {
        Value::String(emoji) => i18n_emoji_prefix_from_items(vec![I18nEmoji {
            fallback: normalize_i18n_emoji(emoji)?,
            custom_emoji_id: None,
        }]),
        Value::Array(items) => i18n_emoji_prefix_from_items(
            items
                .iter()
                .filter_map(i18n_emoji_from_value)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("emojis") {
                return i18n_emoji_prefix_from_items(
                    items
                        .iter()
                        .filter_map(i18n_emoji_from_value)
                        .collect::<Vec<_>>(),
                );
            }
            let fallback = map
                .get("fallback")
                .or_else(|| map.get("emoji"))
                .and_then(Value::as_str)
                .and_then(normalize_i18n_emoji)?;
            let custom_emoji_id = map
                .get("custom_emoji_id")
                .and_then(Value::as_str)
                .and_then(normalize_custom_emoji_id);
            i18n_emoji_prefix_from_items(vec![I18nEmoji {
                fallback,
                custom_emoji_id,
            }])
        }
        _ => None,
    }
}

fn i18n_emoji_from_value(value: &Value) -> Option<I18nEmoji> {
    match value {
        Value::String(emoji) => Some(I18nEmoji {
            fallback: normalize_i18n_emoji(emoji)?,
            custom_emoji_id: None,
        }),
        Value::Object(map) => {
            let fallback = map
                .get("fallback")
                .or_else(|| map.get("emoji"))
                .and_then(Value::as_str)
                .and_then(normalize_i18n_emoji)?;
            let custom_emoji_id = map
                .get("custom_emoji_id")
                .and_then(Value::as_str)
                .and_then(normalize_custom_emoji_id);
            Some(I18nEmoji {
                fallback,
                custom_emoji_id,
            })
        }
        _ => None,
    }
}

fn i18n_emoji_prefix_from_items(items: Vec<I18nEmoji>) -> Option<I18nEmojiPrefix> {
    let emojis = items
        .into_iter()
        .filter(|item| !item.fallback.trim().is_empty())
        .collect::<Vec<_>>();
    if emojis.is_empty() {
        return None;
    }
    let fallback = emojis
        .iter()
        .map(|item| item.fallback.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let custom_emoji_id = emojis.iter().find_map(|item| item.custom_emoji_id.clone());
    Some(I18nEmojiPrefix {
        fallback,
        custom_emoji_id,
        emojis,
    })
}

pub fn custom_emoji_map_from_values(emojis: HashMap<String, Value>) -> HashMap<String, String> {
    emojis
        .into_iter()
        .filter_map(|(fallback, value)| {
            let fallback = normalize_i18n_emoji(&fallback)?;
            let custom_emoji_id = custom_emoji_id_from_value(&value)?;
            Some((fallback, custom_emoji_id))
        })
        .collect()
}

fn custom_emoji_id_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(custom_emoji_id) => normalize_custom_emoji_id(custom_emoji_id),
        Value::Object(map) => map
            .get("custom_emoji_id")
            .and_then(Value::as_str)
            .and_then(normalize_custom_emoji_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{AppContext, config_value_from_map};
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use teloxide::Bot;

    #[test]
    fn config_value_prefers_runtime_config_cache() {
        let configs = HashMap::from([("bank_name".to_string(), "ACB".to_string())]);

        assert_eq!(config_value_from_map(&configs, "bank_name", "VCB"), "ACB");
        assert_eq!(
            config_value_from_map(&configs, "missing", "fallback"),
            "fallback"
        );
    }

    #[tokio::test]
    async fn crypto_feature_helpers_reflect_enabled_methods() {
        let disabled = test_ctx(Config::from_env_map(&required_env()).unwrap());
        assert!(!disabled.usdt_payments_enabled());
        assert!(!disabled.binance_pay_enabled());
        assert!(!disabled.bep20_enabled());

        let mut env = required_env();
        env.insert("BINANCE_PAY_NOTE_ENABLED".to_string(), "true".to_string());
        env.insert("BINANCE_PAY_API_KEY".to_string(), "api-key".to_string());
        env.insert("BINANCE_PAY_API_SECRET".to_string(), "secret".to_string());
        env.insert(
            "BINANCE_PAY_RECEIVER_PAY_ID".to_string(),
            "209378262".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECEIVER_NAME".to_string(),
            "Receiver".to_string(),
        );
        let binance = test_ctx(Config::from_env_map(&env).unwrap());
        assert!(binance.usdt_payments_enabled());
        assert!(binance.binance_pay_enabled());
        assert!(!binance.bep20_enabled());

        let mut env = required_env();
        env.insert(
            "BEP20_MERCHANT_WALLET".to_string(),
            "0x0000000000000000000000000000000000000001".to_string(),
        );
        env.insert("BSCSCAN_API_KEY".to_string(), "bsc-key".to_string());
        let bep20 = test_ctx(Config::from_env_map(&env).unwrap());
        assert!(bep20.usdt_payments_enabled());
        assert!(!bep20.binance_pay_enabled());
        assert!(bep20.bep20_enabled());
    }

    #[tokio::test]
    async fn usdt_rate_cache_starts_empty() {
        let ctx = test_ctx(Config::from_env_map(&required_env()).unwrap());

        assert!(ctx.usdt_rate_cache.read().await.is_none());
    }

    #[tokio::test]
    async fn crypto_feature_helpers_use_runtime_admin_config() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                (
                    "bep20_merchant_wallet".to_string(),
                    "0x0000000000000000000000000000000000000001".to_string(),
                ),
                ("bscscan_api_key".to_string(), "runtime-bsc-key".to_string()),
                ("usd_vnd_fallback_rate".to_string(), "26000".to_string()),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert!(ctx.bep20_enabled());
        assert_eq!(
            ctx.bep20_merchant_wallet().as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );
        assert_eq!(ctx.usd_vnd_fallback_rate().to_string(), "26000");
    }

    #[tokio::test]
    async fn crypto_disabled_reasons_use_admin_config_keys() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(
            ctx.binance_pay_disabled_reason().as_deref(),
            Some(
                "missing admin config: binance_pay_api_key/binance_pay_api_secret/binance_pay_receiver_pay_id/binance_pay_receiver_name"
            )
        );
        assert_eq!(
            ctx.bep20_disabled_reason().as_deref(),
            Some("missing admin config: bep20_merchant_wallet/bscscan_api_key")
        );
    }

    #[tokio::test]
    async fn binance_pay_urls_use_runtime_admin_config() {
        let mut env_config = Config::from_env_map(&required_env()).unwrap();
        env_config.crypto.binance.webhook_url = Some("https://env.test/webhook".to_string());
        env_config.crypto.binance.return_url = Some("https://env.test/return".to_string());
        env_config.crypto.binance.cancel_url = Some("https://env.test/cancel".to_string());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                (
                    "binance_pay_webhook_url".to_string(),
                    "https://admin.test/webhook".to_string(),
                ),
                (
                    "binance_pay_return_url".to_string(),
                    "https://admin.test/return".to_string(),
                ),
                (
                    "binance_pay_cancel_url".to_string(),
                    "https://admin.test/cancel".to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(
            ctx.binance_pay_webhook_url().as_deref(),
            Some("https://admin.test/webhook")
        );
        assert_eq!(
            ctx.binance_pay_return_url().as_deref(),
            Some("https://admin.test/return")
        );
        assert_eq!(
            ctx.binance_pay_cancel_url().as_deref(),
            Some("https://admin.test/cancel")
        );
    }

    #[tokio::test]
    async fn order_memo_config_uses_runtime_admin_values() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("order_memo_prefix".to_string(), "shop".to_string()),
                ("order_memo_length".to_string(), "12".to_string()),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(ctx.order_memo_prefix(), "SHOP");
        assert_eq!(ctx.order_memo_length(), 12);
    }

    #[tokio::test]
    async fn order_memo_prefix_defaults_to_ptn1411() {
        let ctx = test_ctx(Config::from_env_map(&required_env()).unwrap());

        assert_eq!(ctx.order_memo_prefix(), "PTN1411");
    }

    #[tokio::test]
    async fn telegram_icon_admin_ids_accept_commas_newlines_and_spaces() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([(
                "telegram_icon_admin_ids".to_string(),
                "123, 456\n789 invalid".to_string(),
            )]),
            BotTexts::default(),
            vec![],
        );

        assert!(ctx.is_telegram_icon_admin(123));
        assert!(ctx.is_telegram_icon_admin(456));
        assert!(ctx.is_telegram_icon_admin(789));
        assert!(!ctx.is_telegram_icon_admin(111));
    }

    #[tokio::test]
    async fn i18n_emoji_for_key_parses_config_map_and_filters_invalid_values() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":"  ✅  ","bad":"","too_long":"abcdefghijk"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(ctx.i18n_emoji_for_key("help").as_deref(), Some("✅"));
        assert_eq!(ctx.i18n_emoji_for_key("bad"), None);
        assert_eq!(ctx.i18n_emoji_for_key("too_long"), None);
        assert_eq!(ctx.i18n_emoji_for_key("missing"), None);
    }

    #[tokio::test]
    async fn i18n_emoji_prefix_for_key_parses_custom_emoji_config() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":{"fallback":"🔥","custom_emoji_id":"5368324170671202286"},"legacy":"✅"}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let custom = ctx.i18n_emoji_prefix_for_key("help").unwrap();
        assert_eq!(custom.fallback, "🔥");
        assert_eq!(
            custom.custom_emoji_id.as_deref(),
            Some("5368324170671202286")
        );

        let legacy = ctx.i18n_emoji_prefix_for_key("legacy").unwrap();
        assert_eq!(legacy.fallback, "✅");
        assert_eq!(legacy.custom_emoji_id, None);
    }

    #[tokio::test]
    async fn i18n_emoji_for_key_parses_multiple_custom_emoji_prefixes() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":{"emojis":[{"fallback":"🔥","custom_emoji_id":"5368324170671202286"},{"fallback":"🎁","custom_emoji_id":"5368324170671202287"}]}}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(ctx.i18n_emoji_for_key("help").as_deref(), Some("🔥 🎁"));
    }

    #[tokio::test]
    async fn i18n_emojis_are_disabled_until_config_is_enabled() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let emoji_map = (
            "telegram_i18n_emojis".to_string(),
            r#"{"help":"💬"}"#.to_string(),
        );
        let disabled = AppContext::new(
            Bot::new("test-token"),
            pool.clone(),
            env_config.clone(),
            HashMap::from([emoji_map.clone()]),
            BotTexts::default(),
            vec![],
        );
        let enabled = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                emoji_map,
            ]),
            BotTexts::default(),
            vec![],
        );

        assert!(!disabled.i18n_emojis_enabled());
        assert_eq!(disabled.i18n_emoji_for_key("help"), None);
        assert!(enabled.i18n_emojis_enabled());
        assert_eq!(enabled.i18n_emoji_for_key("help").as_deref(), Some("💬"));
    }

    #[tokio::test]
    async fn custom_emoji_map_parses_fallback_to_custom_id_entries() {
        let env_config = Config::from_env_map(&required_env()).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            env_config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_custom_emojis".to_string(),
                    r#"{"🔥":"5368324170671202286","🎁":{"custom_emoji_id":"5368324170671202287"},"bad":"abc"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let map = ctx.custom_emoji_map();

        assert_eq!(
            map.get("🔥").map(String::as_str),
            Some("5368324170671202286")
        );
        assert_eq!(
            map.get("🎁").map(String::as_str),
            Some("5368324170671202287")
        );
        assert_eq!(map.get("bad"), None);
    }

    fn test_ctx(config: Config) -> std::sync::Arc<AppContext> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    fn required_env() -> HashMap<String, String> {
        HashMap::from([
            ("TELOXIDE_TOKEN".to_string(), "test-token".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "test-admin-jwt-secret-at-least-32-chars".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup-code".to_string()),
        ])
    }
}
