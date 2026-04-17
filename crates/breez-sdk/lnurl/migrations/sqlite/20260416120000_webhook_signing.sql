-- Per-domain signing secret for outgoing webhooks.
ALTER TABLE domain_webhooks ADD COLUMN webhook_secret TEXT NOT NULL;

-- Recreate webhook_deliveries with domain column and correct constraints.
-- No existing SQLite databases have webhook data, so a clean drop is safe.
DROP TABLE webhook_deliveries;

CREATE TABLE webhook_deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL,
    domain TEXT NOT NULL,
    url TEXT,
    payload TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    succeeded_at BIGINT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at BIGINT NOT NULL,
    claimed_at BIGINT,
    last_error_status_code INTEGER,
    last_error_body TEXT,
    UNIQUE (identifier, domain)
);
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries (domain, next_retry_at)
    WHERE succeeded_at IS NULL;
CREATE INDEX idx_webhook_deliveries_created_at
    ON webhook_deliveries (created_at);
