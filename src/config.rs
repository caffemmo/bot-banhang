use std::collections::HashMap;
use std::env;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;

#[derive(Debug, Clone)]
pub struct Config {
    pub telegram_token: String,
    pub database_url: String,
    pub bank_name: String,
    pub bank_account: Option<String>,
    pub bank_account_name: Option<String>,
    pub webhook_secret: String,
    pub admin_jwt_secret: String,
    pub admin_setup_code: String,
    pub admin_cookie_secure: bool,
    #[allow(dead_code)]
    pub base_url: Option<String>,
    pub i18n_dir: String,
    pub port: u16,
    pub crypto: CryptoConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinancePayEnv {
    Sandbox,
    Production,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CryptoConfig {
    pub pay_ttl_minutes: i64,
    pub usdt_rate_buffer_percent: Decimal,
    pub usdt_rate_cache_seconds: u64,
    pub usdt_rate_stale_seconds: u64,
    pub usd_vnd_fallback_rate: Decimal,
    pub rate_provider: String,
    pub rate_custom_url: Option<String>,
    pub binance: BinancePayConfig,
    pub bep20: Bep20Config,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BinancePayConfig {
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub secret: Option<String>,
    pub cert_sn: Option<String>,
    pub env: BinancePayEnv,
    pub webhook_public_key_cache_seconds: u64,
    pub webhook_url: Option<String>,
    pub return_url: Option<String>,
    pub cancel_url: Option<String>,
    pub receiver_pay_id: Option<String>,
    pub receiver_name: Option<String>,
    pub note_enabled: bool,
    pub poll_interval_seconds: u64,
    pub history_lookback_minutes: i64,
    pub recv_window_ms: i64,
    pub match_grace_minutes: i64,
    pub note_prefix: String,
    pub note_digits: u8,
    pub amount_tolerance_usdt: Decimal,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Bep20Config {
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub merchant_wallet: Option<String>,
    pub usdt_contract: String,
    pub required_confirmations: i64,
    pub poll_interval_seconds: u64,
    pub start_block: Option<i64>,
    pub bscscan_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let env_map = env::vars().collect::<HashMap<_, _>>();
        Self::from_env_map(&env_map)
    }

    pub fn from_env_map(env: &HashMap<String, String>) -> Result<Self> {
        let telegram_token = required_env(env, "TELOXIDE_TOKEN")
            .ok_or_else(|| anyhow!("TELOXIDE_TOKEN is required"))?;
        let database_url =
            env_value(env, "DATABASE_URL").unwrap_or_else(|| "sqlite://shop.db".to_string());
        let bank_name = env_value(env, "BANK_NAME").unwrap_or_else(|| "VCB".to_string());
        let bank_account = env_value(env, "BANK_ACCOUNT");
        let bank_account_name = env_value(env, "BANK_ACCOUNT_NAME");
        let webhook_secret =
            env_value(env, "WEBHOOK_SECRET").unwrap_or_else(|| "change-me".to_string());
        let admin_jwt_secret = required_env(env, "ADMIN_JWT_SECRET")
            .ok_or_else(|| anyhow!("ADMIN_JWT_SECRET is required"))?;
        if admin_jwt_secret.len() < 32 {
            return Err(anyhow!("ADMIN_JWT_SECRET must be at least 32 chars"));
        }
        let admin_setup_code = required_env(env, "ADMIN_SETUP_CODE")
            .ok_or_else(|| anyhow!("ADMIN_SETUP_CODE is required"))?;
        if admin_setup_code.len() < 8 {
            return Err(anyhow!("ADMIN_SETUP_CODE must be at least 8 chars"));
        }
        let admin_cookie_secure = env_value(env, "ADMIN_COOKIE_SECURE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        let base_url = env_value(env, "BASE_URL");
        let i18n_dir = env_value(env, "I18N_DIR").unwrap_or_else(|| "i18n".to_string());
        let port = env_value(env, "PORT")
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);
        let crypto = CryptoConfig::from_env_map(env);
        Ok(Self {
            telegram_token,
            database_url,
            bank_name,
            bank_account,
            bank_account_name,
            webhook_secret,
            admin_jwt_secret,
            admin_setup_code,
            admin_cookie_secure,
            base_url,
            i18n_dir,
            port,
            crypto,
        })
    }
}

impl CryptoConfig {
    fn from_env_map(env: &HashMap<String, String>) -> Self {
        let pay_ttl_minutes = parse_i64_range(env, "CRYPTO_PAY_TTL_MINUTES", 30, 1, 1440);
        let usdt_rate_buffer_percent = parse_decimal_range(
            env,
            "USDT_RATE_BUFFER_PERCENT",
            Decimal::ONE,
            Decimal::ZERO,
            Decimal::from(10),
        );
        let usdt_rate_cache_seconds = parse_u64(env, "USDT_RATE_CACHE_SECONDS", 300);
        let usdt_rate_stale_seconds = parse_u64(env, "USDT_RATE_STALE_SECONDS", 600);
        let usd_vnd_fallback_rate =
            parse_decimal(env, "USD_VND_FALLBACK_RATE", Decimal::from(25_000));
        let rate_provider = env_value(env, "USDT_RATE_PROVIDER")
            .unwrap_or_else(|| "env_only".to_string())
            .to_lowercase();
        let rate_custom_url = env_value(env, "USDT_RATE_CUSTOM_URL");

        Self {
            pay_ttl_minutes,
            usdt_rate_buffer_percent,
            usdt_rate_cache_seconds,
            usdt_rate_stale_seconds,
            usd_vnd_fallback_rate,
            rate_provider,
            rate_custom_url,
            binance: BinancePayConfig::from_env_map(env),
            bep20: Bep20Config::from_env_map(env),
        }
    }
}

impl Default for CryptoConfig {
    fn default() -> Self {
        Self {
            pay_ttl_minutes: 30,
            usdt_rate_buffer_percent: Decimal::ONE,
            usdt_rate_cache_seconds: 300,
            usdt_rate_stale_seconds: 600,
            usd_vnd_fallback_rate: Decimal::from(25_000),
            rate_provider: "env_only".to_string(),
            rate_custom_url: None,
            binance: BinancePayConfig::default(),
            bep20: Bep20Config::default(),
        }
    }
}

impl BinancePayConfig {
    fn from_env_map(env: &HashMap<String, String>) -> Self {
        let api_key = env_value(env, "BINANCE_PAY_API_KEY");
        let api_secret = env_value(env, "BINANCE_PAY_API_SECRET");
        let secret = env_value(env, "BINANCE_PAY_SECRET");
        let cert_sn = env_value(env, "BINANCE_PAY_CERT_SN");
        let raw_env = env_value(env, "BINANCE_PAY_ENV").unwrap_or_else(|| "sandbox".to_string());
        let env_kind = match raw_env.as_str() {
            "sandbox" => Some(BinancePayEnv::Sandbox),
            "production" => Some(BinancePayEnv::Production),
            _ => None,
        };
        let webhook_url = valid_url_opt(env_value(env, "BINANCE_PAY_WEBHOOK_URL"));
        let return_url = valid_url_opt(env_value(env, "BINANCE_PAY_RETURN_URL"));
        let cancel_url = valid_url_opt(env_value(env, "BINANCE_PAY_CANCEL_URL"));
        let note_enabled = parse_bool(env, "BINANCE_PAY_NOTE_ENABLED", false);
        let receiver_pay_id =
            env_value(env, "BINANCE_PAY_RECEIVER_PAY_ID").filter(|v| is_valid_pay_id(v));
        let receiver_name =
            env_value(env, "BINANCE_PAY_RECEIVER_NAME").filter(|v| (1..=64).contains(&v.len()));
        let note_prefix = parse_note_prefix(env_value(env, "BINANCE_PAY_NOTE_PREFIX"));

        let disabled_reason = if !note_enabled {
            Some("BINANCE_PAY_NOTE_ENABLED is disabled".to_string())
        } else if api_key.is_none() || api_secret.is_none() {
            Some("missing BINANCE_PAY_API_KEY/BINANCE_PAY_API_SECRET".to_string())
        } else if receiver_pay_id.is_none() || receiver_name.is_none() {
            Some(
                "missing or invalid BINANCE_PAY_RECEIVER_PAY_ID/BINANCE_PAY_RECEIVER_NAME"
                    .to_string(),
            )
        } else if env_kind.is_none() {
            Some("invalid BINANCE_PAY_ENV".to_string())
        } else if url_was_invalid(env, "BINANCE_PAY_WEBHOOK_URL", webhook_url.as_ref())
            || url_was_invalid(env, "BINANCE_PAY_RETURN_URL", return_url.as_ref())
            || url_was_invalid(env, "BINANCE_PAY_CANCEL_URL", cancel_url.as_ref())
        {
            Some("invalid Binance Pay URL".to_string())
        } else {
            None
        };

        Self {
            enabled: disabled_reason.is_none(),
            disabled_reason,
            api_key,
            api_secret,
            secret,
            cert_sn,
            env: env_kind.unwrap_or(BinancePayEnv::Sandbox),
            webhook_public_key_cache_seconds: parse_u64(
                env,
                "BINANCE_PAY_WEBHOOK_PUBLIC_KEY_CACHE_SECONDS",
                3600,
            ),
            webhook_url,
            return_url,
            cancel_url,
            receiver_pay_id,
            receiver_name,
            note_enabled,
            poll_interval_seconds: parse_u64_clamped(
                env,
                "BINANCE_PAY_POLL_INTERVAL_SECONDS",
                30,
                15,
                300,
            ),
            history_lookback_minutes: parse_i64_clamped(
                env,
                "BINANCE_PAY_HISTORY_LOOKBACK_MINUTES",
                120,
                10,
                1440,
            ),
            recv_window_ms: parse_i64_clamped(env, "BINANCE_PAY_RECV_WINDOW_MS", 5000, 1000, 60000),
            match_grace_minutes: parse_i64_clamped(
                env,
                "BINANCE_PAY_MATCH_GRACE_MINUTES",
                10,
                0,
                1440,
            ),
            note_prefix,
            note_digits: parse_u64_clamped(env, "BINANCE_PAY_NOTE_DIGITS", 6, 4, 12) as u8,
            amount_tolerance_usdt: parse_decimal_range(
                env,
                "BINANCE_PAY_AMOUNT_TOLERANCE_USDT",
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::from(10),
            ),
        }
    }
}

impl Default for BinancePayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            disabled_reason: Some("BINANCE_PAY_NOTE_ENABLED is disabled".to_string()),
            api_key: None,
            api_secret: None,
            secret: None,
            cert_sn: None,
            env: BinancePayEnv::Sandbox,
            webhook_public_key_cache_seconds: 3600,
            webhook_url: None,
            return_url: None,
            cancel_url: None,
            receiver_pay_id: None,
            receiver_name: None,
            note_enabled: false,
            poll_interval_seconds: 30,
            history_lookback_minutes: 120,
            recv_window_ms: 5000,
            match_grace_minutes: 10,
            note_prefix: "VI".to_string(),
            note_digits: 6,
            amount_tolerance_usdt: Decimal::ZERO,
        }
    }
}

impl Bep20Config {
    fn from_env_map(env: &HashMap<String, String>) -> Self {
        let raw_wallet = env_value(env, "BEP20_MERCHANT_WALLET");
        let raw_contract = env_value(env, "BEP20_USDT_CONTRACT")
            .unwrap_or_else(|| "0x55d398326f99059fF775485246999027B3197955".to_string());
        let api_key = env_value(env, "BSCSCAN_API_KEY");
        let required_confirmations =
            parse_i64_range(env, "BEP20_REQUIRED_CONFIRMATIONS", 12, 1, i64::MAX);
        let poll_interval_seconds = parse_u64(env, "BEP20_POLL_INTERVAL_SECONDS", 15);
        let start_block = env_value(env, "BEP20_START_BLOCK").and_then(|v| v.parse().ok());

        let wallet_valid = raw_wallet.as_deref().is_some_and(is_eth_address);
        let contract_valid = is_eth_address(&raw_contract);
        let disabled_reason = if raw_wallet.is_none() || api_key.is_none() {
            Some("missing BEP20_MERCHANT_WALLET/BSCSCAN_API_KEY".to_string())
        } else if !wallet_valid {
            Some("invalid BEP20_MERCHANT_WALLET".to_string())
        } else if !contract_valid {
            Some("invalid BEP20_USDT_CONTRACT".to_string())
        } else {
            None
        };

        Self {
            enabled: disabled_reason.is_none(),
            disabled_reason,
            merchant_wallet: raw_wallet.filter(|wallet| is_eth_address(wallet)),
            usdt_contract: if contract_valid {
                raw_contract
            } else {
                "0x55d398326f99059fF775485246999027B3197955".to_string()
            },
            required_confirmations,
            poll_interval_seconds,
            start_block,
            bscscan_api_key: api_key,
        }
    }
}

impl Default for Bep20Config {
    fn default() -> Self {
        Self {
            enabled: false,
            disabled_reason: Some("missing BEP20_MERCHANT_WALLET/BSCSCAN_API_KEY".to_string()),
            merchant_wallet: None,
            usdt_contract: "0x55d398326f99059fF775485246999027B3197955".to_string(),
            required_confirmations: 12,
            poll_interval_seconds: 15,
            start_block: None,
            bscscan_api_key: None,
        }
    }
}

pub fn is_eth_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn env_value(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn required_env(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env_value(env, key)
}

fn parse_u64(env: &HashMap<String, String>, key: &str, default: u64) -> u64 {
    env_value(env, key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn parse_bool(env: &HashMap<String, String>, key: &str, default: bool) -> bool {
    env_value(env, key)
        .map(|v| {
            matches!(
                v.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(default)
}

fn parse_u64_clamped(
    env: &HashMap<String, String>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> u64 {
    env_value(env, key)
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn parse_i64_clamped(
    env: &HashMap<String, String>,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
) -> i64 {
    env_value(env, key)
        .and_then(|v| v.parse::<i64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn parse_i64_range(
    env: &HashMap<String, String>,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
) -> i64 {
    env_value(env, key)
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v >= min && *v <= max)
        .unwrap_or(default)
}

fn parse_decimal(env: &HashMap<String, String>, key: &str, default: Decimal) -> Decimal {
    env_value(env, key)
        .and_then(|v| Decimal::from_str(&v).ok())
        .unwrap_or(default)
}

fn parse_decimal_range(
    env: &HashMap<String, String>,
    key: &str,
    default: Decimal,
    min: Decimal,
    max: Decimal,
) -> Decimal {
    let value = parse_decimal(env, key, default);
    if value >= min && value <= max {
        value
    } else {
        default
    }
}

fn valid_url_opt(value: Option<String>) -> Option<String> {
    value.filter(|v| v.starts_with("http://") || v.starts_with("https://"))
}

fn url_was_invalid(
    env: &HashMap<String, String>,
    key: &str,
    parsed_value: Option<&String>,
) -> bool {
    env_value(env, key).is_some() && parsed_value.is_none()
}

fn is_valid_pay_id(value: &str) -> bool {
    (5..=32).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

fn parse_note_prefix(value: Option<String>) -> String {
    let prefix = value
        .map(|v| v.to_ascii_uppercase())
        .filter(|v| (1..=8).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_uppercase()));
    prefix.unwrap_or_else(|| "VI".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bep20_wallet_address_format() {
        assert!(is_eth_address("0x55d398326f99059fF775485246999027B3197955"));
        assert!(is_eth_address("0x0000000000000000000000000000000000000000"));
        assert!(!is_eth_address("0x123"));
        assert!(!is_eth_address("55d398326f99059fF775485246999027B3197955"));
        assert!(!is_eth_address(
            "0x55d398326f99059fF775485246999027B31979ZZ"
        ));
    }

    #[test]
    fn invalid_crypto_values_disable_features_without_breaking_core_config() {
        let mut env = required_test_env();
        env.insert("BINANCE_PAY_API_KEY".to_string(), "api-key".to_string());
        env.insert("BINANCE_PAY_SECRET".to_string(), "secret".to_string());
        env.insert("BINANCE_PAY_CERT_SN".to_string(), "cert-sn".to_string());
        env.insert("BINANCE_PAY_ENV".to_string(), "invalid".to_string());
        env.insert("BEP20_MERCHANT_WALLET".to_string(), "0x123".to_string());
        env.insert("BSCSCAN_API_KEY".to_string(), "bsc-key".to_string());

        let config = Config::from_env_map(&env).unwrap();

        assert_eq!(config.telegram_token, "test-token");
        assert!(!config.crypto.binance.enabled);
        assert!(!config.crypto.bep20.enabled);
    }

    #[test]
    fn binance_pay_note_config_requires_history_credentials_and_receiver_not_cert() {
        let mut env = required_test_env();
        env.insert("BINANCE_PAY_NOTE_ENABLED".to_string(), "true".to_string());
        env.insert("BINANCE_PAY_API_KEY".to_string(), "api-key".to_string());
        env.insert(
            "BINANCE_PAY_API_SECRET".to_string(),
            "api-secret".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECEIVER_PAY_ID".to_string(),
            "00000000".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECEIVER_NAME".to_string(),
            "xxxxxxxx".to_string(),
        );

        let config = Config::from_env_map(&env).unwrap();

        assert!(config.crypto.binance.enabled);
        assert_eq!(config.crypto.binance.api_key.as_deref(), Some("api-key"));
        assert_eq!(
            config.crypto.binance.api_secret.as_deref(),
            Some("api-secret")
        );
        assert_eq!(
            config.crypto.binance.receiver_pay_id.as_deref(),
            Some("00000000")
        );
        assert_eq!(
            config.crypto.binance.receiver_name.as_deref(),
            Some("xxxxxxxx")
        );
        assert!(config.crypto.binance.cert_sn.is_none());
    }

    #[test]
    fn binance_pay_note_config_clamps_operational_values() {
        let mut env = required_test_env();
        env.insert("BINANCE_PAY_NOTE_ENABLED".to_string(), "1".to_string());
        env.insert("BINANCE_PAY_API_KEY".to_string(), "api-key".to_string());
        env.insert(
            "BINANCE_PAY_API_SECRET".to_string(),
            "api-secret".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECEIVER_PAY_ID".to_string(),
            "12345".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECEIVER_NAME".to_string(),
            "Receiver".to_string(),
        );
        env.insert(
            "BINANCE_PAY_POLL_INTERVAL_SECONDS".to_string(),
            "1".to_string(),
        );
        env.insert(
            "BINANCE_PAY_HISTORY_LOOKBACK_MINUTES".to_string(),
            "99999".to_string(),
        );
        env.insert(
            "BINANCE_PAY_RECV_WINDOW_MS".to_string(),
            "99999".to_string(),
        );
        env.insert(
            "BINANCE_PAY_MATCH_GRACE_MINUTES".to_string(),
            "-1".to_string(),
        );
        env.insert("BINANCE_PAY_NOTE_PREFIX".to_string(), "vi".to_string());
        env.insert("BINANCE_PAY_NOTE_DIGITS".to_string(), "99".to_string());

        let config = Config::from_env_map(&env).unwrap();

        assert_eq!(config.crypto.binance.poll_interval_seconds, 15);
        assert_eq!(config.crypto.binance.history_lookback_minutes, 1440);
        assert_eq!(config.crypto.binance.recv_window_ms, 60_000);
        assert_eq!(config.crypto.binance.match_grace_minutes, 0);
        assert_eq!(config.crypto.binance.note_prefix, "VI");
        assert_eq!(config.crypto.binance.note_digits, 12);
    }

    fn required_test_env() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::from([
            ("TELOXIDE_TOKEN".to_string(), "test-token".to_string()),
            (
                "ADMIN_JWT_SECRET".to_string(),
                "test-admin-jwt-secret-at-least-32-chars".to_string(),
            ),
            ("ADMIN_SETUP_CODE".to_string(), "setup-code".to_string()),
        ])
    }
}
