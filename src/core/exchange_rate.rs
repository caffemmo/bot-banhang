#![allow(dead_code)]

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::warn;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RateCache {
    pub rate_vnd_per_usdt: Decimal,
    pub fetched_at: DateTime<Utc>,
    pub previous_rate: Option<Decimal>,
    pub source: String,
}

#[derive(Debug, Clone, Copy)]
pub struct RateConfig {
    pub buffer_percent: Decimal,
    pub cache_seconds: i64,
    pub stale_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct ProviderRate {
    pub rate_vnd_per_usdt: Decimal,
    pub source: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct UsdtRate {
    pub raw_rate_vnd_per_usdt: Decimal,
    pub buffered_rate_vnd_per_usdt: Decimal,
    pub buffer_percent: Decimal,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
    pub rate_jump_warning: bool,
}

#[async_trait]
pub trait UsdtRateProvider: Send + Sync {
    async fn fetch_rate(&self) -> Result<ProviderRate>;
}

#[derive(Debug, Clone)]
pub struct StaticRateProvider {
    pub rate_vnd_per_usdt: Decimal,
    pub source: String,
}

#[derive(Debug, Clone)]
pub enum ConfiguredRateProvider {
    Static(StaticRateProvider),
    HttpJson { url: String },
}

#[async_trait]
impl UsdtRateProvider for ConfiguredRateProvider {
    async fn fetch_rate(&self) -> Result<ProviderRate> {
        match self {
            Self::Static(provider) => provider.fetch_rate().await,
            Self::HttpJson { url } => {
                let value = reqwest::Client::new()
                    .get(url)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Value>()
                    .await?;
                Ok(ProviderRate {
                    rate_vnd_per_usdt: parse_rate_from_json(&value)?,
                    source: url.clone(),
                })
            }
        }
    }
}

#[async_trait]
impl UsdtRateProvider for StaticRateProvider {
    async fn fetch_rate(&self) -> Result<ProviderRate> {
        Ok(ProviderRate {
            rate_vnd_per_usdt: self.rate_vnd_per_usdt,
            source: self.source.clone(),
        })
    }
}

pub fn parse_rate_from_json(value: &Value) -> Result<Decimal> {
    for key in ["rate", "price", "usd_vnd", "vnd"] {
        if let Some(rate) = value.get(key).and_then(decimal_from_json) {
            return Ok(rate);
        }
    }
    if let Some(rate) = value
        .get("tether")
        .and_then(|tether| tether.get("vnd"))
        .and_then(decimal_from_json)
    {
        return Ok(rate);
    }
    Err(anyhow!("USDT rate JSON does not contain rate/price/vnd"))
}

fn decimal_from_json(value: &Value) -> Option<Decimal> {
    match value {
        Value::Number(number) => Decimal::from_str_exact(&number.to_string()).ok(),
        Value::String(text) => text.parse::<Decimal>().ok(),
        _ => None,
    }
}

pub fn apply_rate_buffer(raw_rate: Decimal, buffer_percent: Decimal) -> Decimal {
    raw_rate + (raw_rate * buffer_percent / Decimal::from(100))
}

pub async fn get_usdt_rate_cached<P>(
    provider: &P,
    cache: &RwLock<Option<RateCache>>,
    config: RateConfig,
    now: DateTime<Utc>,
) -> Result<UsdtRate>
where
    P: UsdtRateProvider,
{
    if let Some(cached) = cache.read().await.clone() {
        let age_seconds = now.signed_duration_since(cached.fetched_at).num_seconds();
        if age_seconds <= config.cache_seconds {
            return Ok(rate_from_cache(cached, config, false));
        }
    }

    match provider.fetch_rate().await {
        Ok(provider_rate) => {
            if provider_rate.rate_vnd_per_usdt <= Decimal::ZERO {
                return Err(anyhow!("USDT rate must be positive"));
            }

            let previous = cache.read().await.clone();
            let rate_jump_warning = previous.as_ref().is_some_and(|prev| {
                is_rate_jump(prev.rate_vnd_per_usdt, provider_rate.rate_vnd_per_usdt)
            });
            if rate_jump_warning {
                warn!(
                    "USDT rate changed more than 5%: previous={} new={}",
                    previous.as_ref().unwrap().rate_vnd_per_usdt,
                    provider_rate.rate_vnd_per_usdt
                );
            }

            let new_cache = RateCache {
                rate_vnd_per_usdt: provider_rate.rate_vnd_per_usdt,
                fetched_at: now,
                previous_rate: previous.map(|prev| prev.rate_vnd_per_usdt),
                source: provider_rate.source,
            };
            *cache.write().await = Some(new_cache.clone());
            Ok(rate_from_cache(new_cache, config, rate_jump_warning))
        }
        Err(err) => {
            if let Some(cached) = cache.read().await.clone() {
                let age_seconds = now.signed_duration_since(cached.fetched_at).num_seconds();
                if age_seconds <= config.stale_seconds {
                    warn!("USDT rate provider failed, using cached rate: {err}");
                    return Ok(rate_from_cache(cached, config, false));
                }
            }
            Err(err)
        }
    }
}

pub async fn get_usdt_rate_cached_or_static_fallback<P>(
    provider: &P,
    fallback: StaticRateProvider,
    cache: &RwLock<Option<RateCache>>,
    config: RateConfig,
    now: DateTime<Utc>,
) -> Result<UsdtRate>
where
    P: UsdtRateProvider,
{
    match get_usdt_rate_cached(provider, cache, config, now).await {
        Ok(rate) => Ok(rate),
        Err(err) => {
            warn!(
                "USDT rate provider failed without usable cache, using fallback rate {}: {err}",
                fallback.rate_vnd_per_usdt
            );
            get_usdt_rate_cached(&fallback, cache, config, now).await
        }
    }
}

fn rate_from_cache(cache: RateCache, config: RateConfig, rate_jump_warning: bool) -> UsdtRate {
    UsdtRate {
        raw_rate_vnd_per_usdt: cache.rate_vnd_per_usdt,
        buffered_rate_vnd_per_usdt: apply_rate_buffer(
            cache.rate_vnd_per_usdt,
            config.buffer_percent,
        ),
        buffer_percent: config.buffer_percent,
        source: cache.source,
        fetched_at: cache.fetched_at,
        rate_jump_warning,
    }
}

fn is_rate_jump(previous: Decimal, current: Decimal) -> bool {
    if previous <= Decimal::ZERO {
        return false;
    }
    let delta = (current - previous).abs();
    (delta * Decimal::from(100) / previous) > Decimal::from(5)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use rust_decimal_macros::dec;
    use tokio::sync::RwLock;

    use super::*;

    struct CountingProvider {
        calls: Arc<AtomicUsize>,
        rate: rust_decimal::Decimal,
    }

    #[async_trait]
    impl UsdtRateProvider for CountingProvider {
        async fn fetch_rate(&self) -> Result<ProviderRate> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProviderRate {
                rate_vnd_per_usdt: self.rate,
                source: "test".to_string(),
            })
        }
    }

    struct FailingProvider;

    #[async_trait]
    impl UsdtRateProvider for FailingProvider {
        async fn fetch_rate(&self) -> Result<ProviderRate> {
            Err(anyhow!("provider down"))
        }
    }

    fn cfg() -> RateConfig {
        RateConfig {
            buffer_percent: dec!(1),
            cache_seconds: 300,
            stale_seconds: 600,
        }
    }

    #[test]
    fn rate_buffer_adds_percent_to_raw_rate() {
        assert_eq!(apply_rate_buffer(dec!(25000), dec!(1)), dec!(25250));
    }

    #[test]
    fn parses_rate_from_common_json_shapes() {
        assert_eq!(
            parse_rate_from_json(&serde_json::json!({ "rate": "25500.5" })).unwrap(),
            dec!(25500.5)
        );
        assert_eq!(
            parse_rate_from_json(&serde_json::json!({ "tether": { "vnd": 25400 } })).unwrap(),
            dec!(25400)
        );
    }

    #[tokio::test]
    async fn cache_hit_does_not_call_provider_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingProvider {
            calls: calls.clone(),
            rate: dec!(25000),
        };
        let cache = RwLock::new(None);
        let now = Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap();

        let first = get_usdt_rate_cached(&provider, &cache, cfg(), now)
            .await
            .unwrap();
        let second = get_usdt_rate_cached(
            &provider,
            &cache,
            cfg(),
            now + chrono::Duration::seconds(10),
        )
        .await
        .unwrap();

        assert_eq!(first.buffered_rate_vnd_per_usdt, dec!(25250));
        assert_eq!(second.buffered_rate_vnd_per_usdt, dec!(25250));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_failure_uses_fresh_cache() {
        let cache = RwLock::new(Some(RateCache {
            rate_vnd_per_usdt: dec!(25000),
            fetched_at: Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap(),
            previous_rate: None,
            source: "cached".to_string(),
        }));

        let rate = get_usdt_rate_cached(
            &FailingProvider,
            &cache,
            cfg(),
            Utc.with_ymd_and_hms(2026, 5, 21, 1, 5, 0).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(rate.raw_rate_vnd_per_usdt, dec!(25000));
        assert_eq!(rate.source, "cached");
    }

    #[tokio::test]
    async fn provider_failure_rejects_stale_cache() {
        let cache = RwLock::new(Some(RateCache {
            rate_vnd_per_usdt: dec!(25000),
            fetched_at: Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap(),
            previous_rate: None,
            source: "cached".to_string(),
        }));

        let result = get_usdt_rate_cached(
            &FailingProvider,
            &cache,
            cfg(),
            Utc.with_ymd_and_hms(2026, 5, 21, 1, 11, 0).unwrap(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn provider_failure_uses_static_fallback_when_cache_unavailable() {
        let cache = RwLock::new(None);
        let rate = get_usdt_rate_cached_or_static_fallback(
            &FailingProvider,
            StaticRateProvider {
                rate_vnd_per_usdt: dec!(25500),
                source: "fallback".to_string(),
            },
            &cache,
            cfg(),
            Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(rate.raw_rate_vnd_per_usdt, dec!(25500));
        assert_eq!(rate.buffered_rate_vnd_per_usdt, dec!(25755));
        assert_eq!(rate.source, "fallback");
    }

    #[tokio::test]
    async fn rate_jump_over_five_percent_is_flagged() {
        let provider = CountingProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            rate: dec!(27000),
        };
        let cache = RwLock::new(Some(RateCache {
            rate_vnd_per_usdt: dec!(25000),
            fetched_at: Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap(),
            previous_rate: None,
            source: "cached".to_string(),
        }));

        let rate = get_usdt_rate_cached(
            &provider,
            &cache,
            cfg(),
            Utc.with_ymd_and_hms(2026, 5, 21, 1, 6, 0).unwrap(),
        )
        .await
        .unwrap();

        assert!(rate.rate_jump_warning);
    }
}
