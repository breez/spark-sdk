-- Per-domain signing secret for outgoing webhooks.
ALTER TABLE domain_webhooks ADD COLUMN webhook_secret TEXT NOT NULL;

-- Route webhook deliveries by domain instead of URL.
-- The URL is resolved at send time from domain_webhooks and stored for audit.
ALTER TABLE webhook_deliveries ADD COLUMN domain TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE webhook_deliveries ALTER COLUMN url DROP NOT NULL;

-- Replace old (identifier, url) uniqueness with domain-based constraint.
ALTER TABLE webhook_deliveries DROP CONSTRAINT webhook_deliveries_identifier_url_key;
CREATE UNIQUE INDEX idx_webhook_deliveries_identifier_domain
    ON webhook_deliveries (identifier, domain);

-- Update the pending index to partition by domain instead of url.
DROP INDEX IF EXISTS idx_webhook_deliveries_pending;
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries (domain, next_retry_at)
    WHERE succeeded_at IS NULL;
