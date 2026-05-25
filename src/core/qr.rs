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
