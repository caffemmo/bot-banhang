CREATE TABLE IF NOT EXISTS support_cases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_code TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL,
    user_chat_id INTEGER NOT NULL,
    user_name TEXT,
    username TEXT,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    closed_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_support_cases_one_open_per_chat
    ON support_cases(user_chat_id)
    WHERE status = 'open';

CREATE INDEX IF NOT EXISTS idx_support_cases_status_updated
    ON support_cases(status, updated_at DESC);

CREATE TABLE IF NOT EXISTS support_admin_messages (
    case_id INTEGER NOT NULL REFERENCES support_cases(id) ON DELETE CASCADE,
    admin_chat_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (admin_chat_id, message_id)
);
