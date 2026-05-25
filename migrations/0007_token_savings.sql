CREATE TABLE token_savings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool_name TEXT NOT NULL,
    tokens_used INTEGER NOT NULL,
    tokens_saved INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'unknown',
    model_name TEXT NOT NULL DEFAULT 'unknown'
);
CREATE INDEX idx_savings_tool ON token_savings(tool_name);
CREATE INDEX idx_savings_timestamp ON token_savings(timestamp);
CREATE INDEX idx_savings_agent ON token_savings(agent_id);
