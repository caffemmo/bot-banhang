ALTER TABLE support_cases ADD COLUMN assigned_agent_id INTEGER;
ALTER TABLE support_cases ADD COLUMN assigned_agent_name TEXT;
ALTER TABLE support_cases ADD COLUMN assigned_at TEXT;
ALTER TABLE support_cases ADD COLUMN last_customer_message_at TEXT;
ALTER TABLE support_cases ADD COLUMN last_agent_reply_at TEXT;

CREATE INDEX IF NOT EXISTS idx_support_cases_assignee_status
    ON support_cases(assigned_agent_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS support_case_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES support_cases(id) ON DELETE CASCADE,
    direction TEXT NOT NULL CHECK (direction IN ('customer', 'agent')),
    source_chat_id INTEGER NOT NULL,
    source_message_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_support_case_messages_case_created
    ON support_case_messages(case_id, id DESC);
