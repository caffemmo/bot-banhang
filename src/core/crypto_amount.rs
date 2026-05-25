#![allow(dead_code)]

use std::future::Future;

use anyhow::{Result, anyhow};
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Clone, PartialEq)]
pub struct Bep20Amount {
    pub base: Decimal,
    pub suffix: Decimal,
    pub expected: Decimal,
    pub token_units: String,
    pub display_amount: String,
}

pub fn calculate_usdt_base_amount(amount_vnd: i64, rate_vnd_per_usdt: Decimal) -> Result<Decimal> {
    if amount_vnd <= 0 {
        return Err(anyhow!("amount_vnd must be positive"));
    }
    if rate_vnd_per_usdt <= Decimal::ZERO {
        return Err(anyhow!("rate_vnd_per_usdt must be positive"));
    }

    let amount = Decimal::from(amount_vnd) / rate_vnd_per_usdt;
    Ok(ceil_decimal_to_scale(amount, 2))
}

pub fn ceil_decimal_to_scale(value: Decimal, scale: u32) -> Decimal {
    let factor = decimal_pow10(scale);
    ((value * factor).ceil() / factor).round_dp(scale)
}

pub fn decimal_to_token_units(amount: Decimal, decimals: u32) -> Result<String> {
    if amount < Decimal::ZERO {
        return Err(anyhow!("amount cannot be negative"));
    }
    let factor = decimal_pow10(decimals);
    let units = amount * factor;
    if units.fract() != Decimal::ZERO {
        return Err(anyhow!("amount has more precision than token decimals"));
    }
    units
        .to_i128()
        .map(|v| v.to_string())
        .ok_or_else(|| anyhow!("token units overflow"))
}

pub fn token_units_to_decimal_string(units: &str, decimals: u32) -> Result<String> {
    let units = units
        .parse::<i128>()
        .map_err(|_| anyhow!("invalid token units"))?;
    if units < 0 {
        return Err(anyhow!("token units cannot be negative"));
    }
    Ok(Decimal::from_i128_with_scale(units, decimals)
        .normalize()
        .to_string())
}

pub async fn generate_bep20_unique_amount<F, Fut>(base: Decimal, is_taken: F) -> Result<Bep20Amount>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    if base <= Decimal::ZERO {
        return Err(anyhow!("base amount must be positive"));
    }

    for _ in 0..20 {
        let suffix_int: i64 = rand::thread_rng().gen_range(1..=9_999);
        let suffix = Decimal::new(suffix_int, 6);
        let expected = base + suffix;
        let token_units = decimal_to_token_units(expected, 18)?;
        if !is_taken(token_units.clone()).await? {
            return Ok(Bep20Amount {
                base,
                suffix,
                expected,
                token_units,
                display_amount: format!("{expected:.6}"),
            });
        }
    }

    Err(anyhow!(
        "Hiện có nhiều giao dịch USDT đang chờ, vui lòng thử lại sau."
    ))
}

fn decimal_pow10(scale: u32) -> Decimal {
    Decimal::from(10_i128.pow(scale))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn base_amount_rounds_up_to_two_decimals() {
        assert_eq!(
            calculate_usdt_base_amount(62_500, dec!(25250)).unwrap(),
            dec!(2.48)
        );
        assert_eq!(
            calculate_usdt_base_amount(50_000, dec!(25000)).unwrap(),
            dec!(2.00)
        );
    }

    #[test]
    fn token_units_convert_exactly() {
        assert_eq!(
            decimal_to_token_units(dec!(2.483842), 18).unwrap(),
            "2483842000000000000"
        );
        assert_eq!(
            token_units_to_decimal_string("2483842000000000000", 18).unwrap(),
            "2.483842"
        );
    }

    #[tokio::test]
    async fn unique_bep20_amount_has_suffix_under_one_cent() {
        let amount = generate_bep20_unique_amount(dec!(2.48), |_| async { Ok(false) })
            .await
            .unwrap();

        assert!(amount.suffix >= dec!(0.000001));
        assert!(amount.suffix <= dec!(0.009999));
        assert!(amount.expected > amount.base);
        assert_eq!(amount.display_amount, format!("{:.6}", amount.expected));
    }

    #[tokio::test]
    async fn unique_bep20_amount_retries_duplicate_token_units() {
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let seen_for_check = seen.clone();
        let amount = generate_bep20_unique_amount(dec!(2.48), move |units| {
            let seen = seen_for_check.clone();
            async move {
                let mut seen = seen.lock().unwrap();
                if seen.is_empty() {
                    seen.insert(units);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(amount.expected > dec!(2.48));
    }
}
