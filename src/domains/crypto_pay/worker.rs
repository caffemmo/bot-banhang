#![allow(dead_code)]

use anyhow::{Result, anyhow};
use serde::Deserialize;

use crate::app::AppContext;
use crate::domains::crypto_pay::models::CryptoPaymentRequest;
use crate::domains::crypto_pay::repo as crypto_repo;
use crate::domains::orders::fulfillment::{PaymentSource, fulfill_paid_order};
use crate::domains::wallet::repo as wallet_repo;

const ETHERSCAN_V2_API_URL: &str = "https://api.etherscan.io/v2/api";
const BNB_SMART_CHAIN_ID: &str = "56";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BscScanTokenTx {
    pub hash: String,
    pub from: String,
    pub to: String,
    #[serde(rename = "contractAddress")]
    pub contract_address: String,
    pub value: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bep20TransferDecision {
    Confirming,
    Complete,
    ManualReview,
}

pub fn is_matching_bep20_transfer(
    tx: &BscScanTokenTx,
    merchant_wallet: &str,
    usdt_contract: &str,
    amount_token_units: &str,
) -> bool {
    tx.contract_address.eq_ignore_ascii_case(usdt_contract)
        && tx.to.eq_ignore_ascii_case(merchant_wallet)
        && tx.value == amount_token_units
}

pub fn confirmations_for(latest_block: i64, tx_block: i64) -> i64 {
    if latest_block < tx_block {
        0
    } else {
        latest_block - tx_block + 1
    }
}

pub fn classify_confirmations(
    confirmations: i64,
    required_confirmations: i64,
    expired: bool,
) -> Bep20TransferDecision {
    if expired {
        Bep20TransferDecision::ManualReview
    } else if confirmations >= required_confirmations {
        Bep20TransferDecision::Complete
    } else {
        Bep20TransferDecision::Confirming
    }
}

#[derive(Debug, Deserialize)]
struct BscScanListResponse {
    status: String,
    message: String,
    result: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct BscScanBlockResponse {
    result: String,
}

#[derive(Debug, Clone)]
pub struct BscScanClient {
    api_key: String,
    http: reqwest::Client,
    base_url: String,
}

impl BscScanClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::new(),
            base_url: ETHERSCAN_V2_API_URL.to_string(),
        }
    }

    async fn latest_block(&self) -> Result<i64> {
        let response = self
            .http
            .get(&self.base_url)
            .query(&[
                ("chainid", BNB_SMART_CHAIN_ID),
                ("module", "proxy"),
                ("action", "eth_blockNumber"),
                ("apikey", self.api_key.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<BscScanBlockResponse>()
            .await?;
        let hex = response.result.trim_start_matches("0x");
        i64::from_str_radix(hex, 16).map_err(Into::into)
    }

    async fn token_transfers(
        &self,
        address: &str,
        contract: &str,
        start_block: i64,
        end_block: i64,
    ) -> Result<Vec<BscScanTokenTx>> {
        let response = self
            .http
            .get(&self.base_url)
            .query(&[
                ("chainid", BNB_SMART_CHAIN_ID),
                ("module", "account"),
                ("action", "tokentx"),
                ("address", address),
                ("contractaddress", contract),
                ("startblock", &start_block.to_string()),
                ("endblock", &end_block.to_string()),
                ("sort", "asc"),
                ("apikey", self.api_key.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<BscScanListResponse>()
            .await?;

        if response.status == "0"
            && response
                .message
                .eq_ignore_ascii_case("No transactions found")
        {
            return Ok(Vec::new());
        }
        if !response.status.eq("1") {
            return Err(anyhow!("Etherscan V2 tokentx failed: {}", response.message));
        }
        serde_json::from_value(response.result).map_err(Into::into)
    }
}

pub async fn run_bep20_tick(ctx: std::sync::Arc<AppContext>) -> Result<()> {
    if !ctx.bep20_enabled() {
        return Ok(());
    }
    let api_key = ctx
        .bep20_bscscan_api_key()
        .ok_or_else(|| anyhow!("BSCSCAN_API_KEY is missing"))?;
    let client = BscScanClient::new(api_key);
    run_bep20_tick_with_client(ctx, &client).await
}

async fn run_bep20_tick_with_client(
    ctx: std::sync::Arc<AppContext>,
    client: &BscScanClient,
) -> Result<()> {
    let wallet = ctx
        .bep20_merchant_wallet()
        .ok_or_else(|| anyhow!("BEP20 merchant wallet is missing"))?;
    let contract = ctx.bep20_usdt_contract();
    let required_confirmations = ctx.bep20_required_confirmations();
    let latest_block = client.latest_block().await?;
    let start_block = ctx
        .bep20_start_block()
        .unwrap_or_else(|| (latest_block - 5000).max(0));
    let safety_window = (required_confirmations + 20).max(50);
    let last_scanned_block =
        crypto_repo::get_worker_state(&ctx.pool, "bep20_last_scanned_block").await?;
    let effective_start = last_scanned_block
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok())
        .map(|last| (last - safety_window).max(start_block))
        .unwrap_or(start_block);

    let txs = client
        .token_transfers(&wallet, &contract, effective_start, latest_block)
        .await?;
    let payments = crypto_repo::list_bep20_payments_for_scan(&ctx.pool).await?;
    process_bep20_transfers(ctx.clone(), &payments, &txs, latest_block).await?;
    crypto_repo::set_worker_state(
        &ctx.pool,
        "bep20_last_scanned_block",
        &latest_block.to_string(),
    )
    .await?;
    Ok(())
}

async fn process_bep20_transfers(
    ctx: std::sync::Arc<AppContext>,
    payments: &[CryptoPaymentRequest],
    txs: &[BscScanTokenTx],
    latest_block: i64,
) -> Result<()> {
    let wallet = ctx
        .bep20_merchant_wallet()
        .ok_or_else(|| anyhow!("BEP20 merchant wallet is missing"))?;
    let contract = ctx.bep20_usdt_contract();
    let required_confirmations = ctx.bep20_required_confirmations();
    for payment in payments {
        let Some(tx) = txs.iter().find(|tx| {
            is_matching_bep20_transfer(tx, &wallet, &contract, &payment.amount_token_units)
        }) else {
            continue;
        };
        let block_number = tx.block_number.parse::<i64>().unwrap_or(0);
        let confirmations = confirmations_for(latest_block, block_number);
        let expired = chrono::DateTime::parse_from_rfc3339(&payment.expires_at)
            .map(|expires_at| chrono::Utc::now() > expires_at.with_timezone(&chrono::Utc))
            .unwrap_or(false);
        match classify_confirmations(confirmations, required_confirmations, expired) {
            Bep20TransferDecision::Confirming => {
                crypto_repo::mark_crypto_payment_confirming(
                    &ctx.pool,
                    payment.id,
                    &tx.hash,
                    &tx.from,
                    block_number,
                    confirmations,
                )
                .await?;
            }
            Bep20TransferDecision::Complete => {
                let mut db_tx = ctx.pool.begin().await?;
                let completed = crypto_repo::complete_crypto_payment(
                    &mut db_tx,
                    payment.id,
                    Some(&tx.hash),
                    confirmations,
                )
                .await?;
                db_tx.commit().await?;
                if completed {
                    if let Some(order_id) = &payment.order_id {
                        fulfill_paid_order(
                            ctx.clone(),
                            order_id,
                            &tx.hash,
                            chrono::Utc::now(),
                            PaymentSource::Bep20 {
                                tx_hash: tx.hash.clone(),
                            },
                        )
                        .await?;
                    } else if payment.purpose == "wallet_topup" {
                        let mut wallet_tx = ctx.pool.begin().await?;
                        wallet_repo::credit_wallet(
                            wallet_tx.as_mut(),
                            payment.user_id,
                            payment.amount_vnd,
                            "topup",
                            None,
                            payment.wallet_topup_id.or(Some(payment.id)),
                            Some("usdt_bep20"),
                        )
                        .await?;
                        wallet_tx.commit().await?;
                    }
                }
            }
            Bep20TransferDecision::ManualReview => {
                crypto_repo::mark_crypto_payment_manual_review(
                    &ctx.pool,
                    payment.id,
                    &format!("late BEP20 transfer detected: {}", tx.hash),
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(contract: &str, to: &str, value: &str, block: &str) -> BscScanTokenTx {
        BscScanTokenTx {
            hash: "0xtx".to_string(),
            from: "0xfrom".to_string(),
            to: to.to_string(),
            contract_address: contract.to_string(),
            value: value.to_string(),
            block_number: block.to_string(),
        }
    }

    #[test]
    fn matching_transfer_requires_contract_recipient_and_exact_units() {
        let contract = "0x55d398326f99059ff775485246999027b3197955";
        let wallet = "0x0000000000000000000000000000000000000001";
        let units = "2483842000000000000";

        assert!(is_matching_bep20_transfer(
            &tx(contract, wallet, units, "100"),
            wallet,
            contract,
            units
        ));
        assert!(!is_matching_bep20_transfer(
            &tx("0xwrong", wallet, units, "100"),
            wallet,
            contract,
            units
        ));
        assert!(!is_matching_bep20_transfer(
            &tx(
                contract,
                "0x0000000000000000000000000000000000000002",
                units,
                "100"
            ),
            wallet,
            contract,
            units
        ));
        assert!(!is_matching_bep20_transfer(
            &tx(contract, wallet, "2483843000000000000", "100"),
            wallet,
            contract,
            units
        ));
    }

    #[test]
    fn confirmations_are_latest_minus_tx_block_plus_one() {
        assert_eq!(confirmations_for(120, 120), 1);
        assert_eq!(confirmations_for(120, 100), 21);
        assert_eq!(confirmations_for(100, 120), 0);
    }

    #[test]
    fn classifies_confirming_and_complete_transfers() {
        assert_eq!(
            classify_confirmations(3, 12, false),
            Bep20TransferDecision::Confirming
        );
        assert_eq!(
            classify_confirmations(12, 12, false),
            Bep20TransferDecision::Complete
        );
        assert_eq!(
            classify_confirmations(15, 12, true),
            Bep20TransferDecision::ManualReview
        );
    }

    #[test]
    fn bep20_client_uses_etherscan_v2_bnb_chain() {
        let client = BscScanClient::new("key".to_string());

        assert_eq!(client.base_url, ETHERSCAN_V2_API_URL);
        assert_eq!(BNB_SMART_CHAIN_ID, "56");
    }
}
