CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

ALTER TABLE newly_paid ADD COLUMN claimed_by TEXT;
ALTER TABLE newly_paid ADD COLUMN claimed_at BIGINT;
