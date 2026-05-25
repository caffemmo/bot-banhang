use anyhow::{Result, anyhow};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::Sha256;

use crate::app::AppContext;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq)]
pub struct BinancePayTransaction {
    pub provider_tx_id: Option<String>,
    pub provider_order_id: Option<String>,
    pub note: Option<String>,
    pub amount: Decimal,
    pub currency: String,
    pub transaction_time_ms: i64,
    pub status: Option<String>,
    pub direction: Option<String>,
    pub raw_json: Value,
}

#[derive(Clone)]
pub struct BinancePayHistoryClient {
    api_key: String,
    api_secret: String,
    base_url: String,
    recv_window_ms: i64,
    http: reqwest::Client,
}

impl BinancePayHistoryClient {
    pub fn from_context(ctx: &AppContext) -> Result<Self> {
        Ok(Self {
            api_key: ctx
                .binance_pay_api_key()
                .ok_or_else(|| anyhow!("BINANCE_PAY_API_KEY is missing"))?,
            api_secret: ctx
                .binance_pay_api_secret()
                .ok_or_else(|| anyhow!("BINANCE_PAY_API_SECRET is missing"))?,
            base_url: "https://api.binance.com".to_string(),
            recv_window_ms: ctx.binance_pay_recv_window_ms(),
            http: reqwest::Client::new(),
        })
    }

    pub async fn list_transactions(
        &self,
        start_time_ms: i64,
        end_time_ms: i64,
        limit: u16,
    ) -> Result<Vec<BinancePayTransaction>> {
        let limit = limit.clamp(1, 100);
        let timestamp = Utc::now().timestamp_millis();
        let query = format!(
            "timestamp={timestamp}&startTime={start_time_ms}&endTime={end_time_ms}&limit={limit}&recvWindow={}",
            self.recv_window_ms
        );
        let signature = sign_query(&query, &self.api_secret);
        let url = format!(
            "{}/sapi/v1/pay/transactions?{}&signature={}",
            self.base_url, query, signature
        );
        let raw = self
            .http
            .get(url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        parse_transactions_response(raw)
    }
}

pub fn sign_query(query: &str, secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(query.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn parse_transactions_response(raw: Value) -> Result<Vec<BinancePayTransaction>> {
    let data = match raw {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("data") {
            Some(Value::Array(items)) => items,
            Some(_) => return Err(anyhow!("Binance Pay history data is not an array")),
            None => return Err(anyhow!("Binance Pay history response missing data")),
        },
        _ => {
            return Err(anyhow!(
                "Binance Pay history response must be object or array"
            ));
        }
    };

    data.into_iter()
        .filter_map(|item| normalize_pay_transaction(item).transpose())
        .collect()
}

pub fn normalize_pay_transaction(raw: Value) -> Result<Option<BinancePayTransaction>> {
    let Some(amount) = decimal_alias(&raw, &["amount", "orderAmount"])
        .or_else(|| nested_decimal_alias(&raw, "fundsDetail", &["amount"]))
    else {
        return Ok(None);
    };
    let Some(currency) = string_alias(&raw, &["currency", "fiatCurrency", "cryptoCurrency"])
        .or_else(|| nested_string_alias(&raw, "fundsDetail", &["currency"]))
    else {
        return Ok(None);
    };
    let Some(transaction_time_ms) = i64_alias(
        &raw,
        &["transactionTime", "time", "createTime", "transactTime"],
    ) else {
        return Ok(None);
    };

    Ok(Some(BinancePayTransaction {
        provider_tx_id: string_alias(&raw, &["transactionId", "transaction_id"]),
        provider_order_id: string_alias(&raw, &["orderId", "order_id"]),
        note: string_alias(&raw, &["note", "remark"])
            .or_else(|| nested_string_alias(&raw, "payerInfo", &["note"])),
        amount,
        currency,
        transaction_time_ms,
        status: string_alias(&raw, &["status"]),
        direction: string_alias(&raw, &["direction", "orderType", "transactionType"]),
        raw_json: raw,
    }))
}

fn string_alias(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        raw.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })
}

fn nested_string_alias(raw: &Value, parent: &str, keys: &[&str]) -> Option<String> {
    raw.get(parent)
        .and_then(|nested| string_alias(nested, keys))
}

fn decimal_alias(raw: &Value, keys: &[&str]) -> Option<Decimal> {
    keys.iter().find_map(|key| {
        raw.get(*key).and_then(|value| match value {
            Value::String(text) => text.parse::<Decimal>().ok(),
            Value::Number(number) => number.to_string().parse::<Decimal>().ok(),
            _ => None,
        })
    })
}

fn nested_decimal_alias(raw: &Value, parent: &str, keys: &[&str]) -> Option<Decimal> {
    raw.get(parent)
        .and_then(|nested| decimal_alias(nested, keys))
}

fn i64_alias(raw: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        raw.get(*key).and_then(|value| match value {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.parse::<i64>().ok(),
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn signs_spot_query_with_lowercase_hmac_sha256() {
        let signature = sign_query(
            "timestamp=1716000000000&startTime=1715999900000&limit=100",
            "test-secret",
        );

        assert_eq!(
            signature,
            "80ca5dd41e4f37b95c9716bf8023eedb4fe788db44f13752d59bf6f4a67afd6e"
        );
    }

    #[test]
    fn parses_wrapped_data_array_with_field_aliases() {
        let txs = parse_transactions_response(json!({
            "code": "000000",
            "message": "success",
            "success": true,
            "data": [{
                "transactionId": "tx-1",
                "orderId": "order-1",
                "transactionTime": 1716000000000i64,
                "amount": "1.25",
                "currency": "USDT",
                "note": "VI265397",
                "status": "SUCCESS",
                "orderType": "PAY"
            }]
        }))
        .unwrap();

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].provider_tx_id.as_deref(), Some("tx-1"));
        assert_eq!(txs[0].provider_order_id.as_deref(), Some("order-1"));
        assert_eq!(txs[0].note.as_deref(), Some("VI265397"));
        assert_eq!(txs[0].amount, dec!(1.25));
        assert_eq!(txs[0].currency, "USDT");
        assert_eq!(txs[0].transaction_time_ms, 1716000000000);
    }

    #[test]
    fn parses_top_level_array_and_nested_funds_detail() {
        let txs = parse_transactions_response(json!([{
            "transaction_id": "tx-2",
            "createTime": 1716000001000i64,
            "remark": "VI000001",
            "fundsDetail": {
                "amount": "2.5",
                "currency": "USDT"
            }
        }]))
        .unwrap();

        assert_eq!(txs[0].provider_tx_id.as_deref(), Some("tx-2"));
        assert_eq!(txs[0].note.as_deref(), Some("VI000001"));
        assert_eq!(txs[0].amount, dec!(2.5));
        assert_eq!(txs[0].currency, "USDT");
    }

    #[test]
    fn keeps_missing_note_as_none_for_matcher_to_ignore() {
        let tx = normalize_pay_transaction(json!({
            "transactionId": "tx-3",
            "transactionTime": 1716000002000i64,
            "amount": "3",
            "currency": "USDT"
        }))
        .unwrap()
        .unwrap();

        assert_eq!(tx.note, None);
        assert_eq!(tx.amount, dec!(3));
    }

    #[test]
    fn skips_records_missing_required_amount_currency_or_time() {
        let tx = normalize_pay_transaction(json!({
            "transactionId": "tx-4",
            "note": "VI000002",
            "currency": "USDT"
        }))
        .unwrap();

        assert!(tx.is_none());
    }
}
