CREATE TABLE IF NOT EXISTS vip_tuts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    teaser TEXT NOT NULL,
    content TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    view_count INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    posted_at TEXT,
    posted_chat_id TEXT,
    posted_message_id INTEGER
);

CREATE TABLE IF NOT EXISTS vip_tut_memberships (
    user_id INTEGER PRIMARY KEY,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS vip_tut_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tut_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    viewed_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_vip_tut_views_tut_user
    ON vip_tut_views(tut_id, user_id);
