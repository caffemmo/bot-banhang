-- Số dư ví theo user
CREATE TABLE IF NOT EXISTS wallets (
    user_id    INTEGER PRIMARY KEY,
    balance    INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Lịch sử giao dịch ví (audit trail)
CREATE TABLE IF NOT EXISTS wallet_transactions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL,
    type          TEXT NOT NULL CHECK(type IN ('topup','purchase','refund','admin_adjust')),
    amount        INTEGER NOT NULL,         -- dương = cộng, âm = trừ
    balance_after INTEGER NOT NULL,         -- snapshot sau giao dịch
    order_id      TEXT,                     -- FK → orders.id (purchase/refund)
    topup_id      INTEGER,                  -- FK → wallet_topup_requests.id
    note          TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_wallet_tx_user ON wallet_transactions(user_id, created_at);

-- Yêu cầu nạp tiền đang chờ
CREATE TABLE IF NOT EXISTS wallet_topup_requests (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL,
    chat_id      INTEGER NOT NULL,
    amount       INTEGER NOT NULL,
    memo         TEXT NOT NULL UNIQUE,    -- "NAP" + 8 ký tự in hoa
    status       TEXT NOT NULL DEFAULT 'pending'
                     CHECK(status IN ('pending','completed','expired')),
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
CREATE INDEX idx_topup_memo   ON wallet_topup_requests(memo);
CREATE INDEX idx_topup_status ON wallet_topup_requests(user_id, status);
