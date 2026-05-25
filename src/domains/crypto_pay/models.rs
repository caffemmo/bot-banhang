use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoPaymentMethod {
    BinancePay,
    Bep20,
}

impl fmt::Display for CryptoPaymentMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BinancePay => "binance_pay",
            Self::Bep20 => "bep20",
        })
    }
}

impl FromStr for CryptoPaymentMethod {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "binance_pay" => Ok(Self::BinancePay),
            "bep20" => Ok(Self::Bep20),
            _ => Err(anyhow!("invalid crypto payment method: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoPaymentStatus {
    Pending,
    Confirming,
    Completed,
    Expired,
    Failed,
    ManualReview,
}

impl fmt::Display for CryptoPaymentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "pending",
            Self::Confirming => "confirming",
            Self::Completed => "completed",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::ManualReview => "manual_review",
        })
    }
}

impl FromStr for CryptoPaymentStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "confirming" => Ok(Self::Confirming),
            "completed" => Ok(Self::Completed),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            "manual_review" => Ok(Self::ManualReview),
            _ => Err(anyhow!("invalid crypto payment status: {value}")),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoPaymentRequest {
    pub id: i64,
    pub purpose: String,
    pub order_id: Option<String>,
    pub wallet_topup_id: Option<i64>,
    pub user_id: i64,
    pub chat_id: i64,
    pub method: CryptoPaymentMethod,
    pub amount_vnd: i64,
    pub rate_vnd_per_usdt: Decimal,
    pub amount_usdt_base: Decimal,
    pub amount_usdt_expected: Decimal,
    pub amount_token_units: String,
    pub memo: String,
    pub address: Option<String>,
    pub binance_prepay_id: Option<String>,
    pub binance_checkout_url: Option<String>,
    pub binance_qrcode_link: Option<String>,
    pub binance_qr_content: Option<String>,
    pub binance_deeplink: Option<String>,
    pub binance_universal_url: Option<String>,
    pub binance_transaction_id: Option<String>,
    pub binance_open_user_id: Option<String>,
    pub tx_hash: Option<String>,
    pub tx_from: Option<String>,
    pub tx_block_number: Option<i64>,
    pub confirmations: i64,
    pub status: CryptoPaymentStatus,
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub completed_at: Option<String>,
    pub updated_at: String,
}
