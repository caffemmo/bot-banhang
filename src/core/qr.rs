use anyhow::{anyhow, Context, Result};
use urlencoding::encode;

pub fn vietqr_link(bank_code: &str, account: &str, amount: i64, memo: &str) -> String {
    // According to https://img.vietqr.io docs.
    // Format: https://img.vietqr.io/image/{bankCode}-{account}-qr_only.png?amount=...&addInfo=...
    format!(
        "https://img.vietqr.io/image/{bank}-{acct}-print.png?amount={amount}&addInfo={memo}",
        bank = encode(bank_code),
        acct = encode(account),
        amount = amount,
        memo = encode(memo)
    )
}

pub async fn download_vietqr_image(url: &str) -> Result<Vec<u8>> {
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to request VietQR image: {url}"))?
        .error_for_status()
        .with_context(|| format!("VietQR returned an error status: {url}"))?;

    let is_image = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false);
    if !is_image {
        return Err(anyhow!("VietQR response is not an image: {url}"));
    }

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read VietQR image bytes: {url}"))?;
    if bytes.is_empty() {
        return Err(anyhow!("VietQR image is empty: {url}"));
    }

    Ok(bytes.to_vec())
}
