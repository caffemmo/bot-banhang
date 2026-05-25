CREATE TABLE IF NOT EXISTS binance_pay_transactions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_tx_id      TEXT,
    provider_order_id   TEXT,
    provider_raw_id     TEXT,
    note                TEXT,
    amount_usdt         TEXT,
    currency            TEXT,
    transaction_time_ms INTEGER,
    status              TEXT,
    direction           TEXT,
    raw_json            TEXT NOT NULL,
    first_seen_at       TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at        TEXT NOT NULL DEFAULT (datetime('now')),
    matched_payment_id  INTEGER REFERENCES crypto_payment_requests(id),
    match_status        TEXT NOT NULL DEFAULT 'unmatched'
                        CHECK(match_status IN (
                            'unmatched',
                            'matched',
                            'ignored',
                            'manual_review',
                            'invalid'
                        )),
    match_reason        TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_binance_pay_transactions_provider_tx_id
ON binance_pay_transactions(provider_tx_id)
WHERE provider_tx_id IS NOT NULL AND provider_tx_id != '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_binance_pay_transactions_provider_raw_id
ON binance_pay_transactions(provider_raw_id)
WHERE provider_raw_id IS NOT NULL AND provider_raw_id != '';

CREATE INDEX IF NOT EXISTS idx_binance_pay_transactions_note
ON binance_pay_transactions(note);

CREATE INDEX IF NOT EXISTS idx_binance_pay_transactions_time
ON binance_pay_transactions(transaction_time_ms);

CREATE INDEX IF NOT EXISTS idx_binance_pay_transactions_match
ON binance_pay_transactions(match_status, matched_payment_id);
