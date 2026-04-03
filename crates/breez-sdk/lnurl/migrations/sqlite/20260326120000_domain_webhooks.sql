-- Webhook URL configured per domain (manually populated).
CREATE TABLE domain_webhooks (
    domain VARCHAR(255) PRIMARY KEY,
    url TEXT NOT NULL
);

-- Track each webhook delivery attempt for guaranteed delivery.
-- Stores the complete payload so the webhook domain is self-contained
-- and does not need to query LNURL tables at delivery time.
CREATE TABLE webhook_deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL,
    url TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    succeeded_at BIGINT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at BIGINT NOT NULL,
    claimed_at BIGINT,
    last_error_status_code INTEGER,
    last_error_body TEXT,
    UNIQUE (identifier, url)
);
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries (url, next_retry_at)
    WHERE succeeded_at IS NULL;
CREATE INDEX idx_webhook_deliveries_created_at
    ON webhook_deliveries (created_at);
