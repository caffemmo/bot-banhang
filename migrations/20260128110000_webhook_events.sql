-- Webhook audit log events (SePay + legacy)
CREATE TABLE IF NOT EXISTS webhook_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  provider TEXT NOT NULL,
  authorized INTEGER NOT NULL DEFAULT 0,
  source_ip TEXT,
  memo_extracted TEXT,
  tx_id TEXT,
  amount INTEGER,
  status TEXT,
  matched_order_id TEXT,
  result TEXT,
  error TEXT,
  raw_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_webhook_events_received_at ON webhook_events(received_at);
CREATE INDEX IF NOT EXISTS idx_webhook_events_tx_id ON webhook_events(tx_id);
CREATE INDEX IF NOT EXISTS idx_webhook_events_memo ON webhook_events(memo_extracted);
CREATE INDEX IF NOT EXISTS idx_webhook_events_order ON webhook_events(matched_order_id);
