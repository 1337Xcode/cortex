-- Enhanced token savings with baseline computation
ALTER TABLE token_savings ADD COLUMN baseline_cost INTEGER NOT NULL DEFAULT 0;
ALTER TABLE token_savings ADD COLUMN net_saved INTEGER NOT NULL DEFAULT 0;
ALTER TABLE token_savings ADD COLUMN query_terms TEXT NOT NULL DEFAULT '[]';
