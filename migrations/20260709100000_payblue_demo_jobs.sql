CREATE TABLE IF NOT EXISTS payblue_demo_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payblue_demo_jobs_user_created
    ON payblue_demo_jobs (user_id, created_at);

CREATE INDEX IF NOT EXISTS idx_payblue_demo_jobs_status
    ON payblue_demo_jobs (status, created_at);
