CREATE TABLE IF NOT EXISTS telegram_update_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id INTEGER,
  user_id INTEGER,
  update_type TEXT NOT NULL,
  raw_json TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_telegram_update_logs_chat_id ON telegram_update_logs(chat_id);
CREATE INDEX IF NOT EXISTS idx_telegram_update_logs_created_at ON telegram_update_logs(created_at);

CREATE TABLE IF NOT EXISTS telegram_chat_messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id INTEGER NOT NULL,
  user_id INTEGER,
  direction TEXT NOT NULL,
  text TEXT,
  telegram_message_id INTEGER,
  telegram_date TEXT,
  raw_json TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_telegram_chat_messages_chat_id ON telegram_chat_messages(chat_id);
CREATE INDEX IF NOT EXISTS idx_telegram_chat_messages_created_at ON telegram_chat_messages(created_at);
