-- This table specifically tracks zap receipt publishing, not generic payment events.
ALTER TABLE newly_paid RENAME TO pending_zap_receipts;
DROP INDEX IF EXISTS idx_newly_paid_next_retry_at;
CREATE INDEX idx_pending_zap_receipts_next_retry_at ON pending_zap_receipts (next_retry_at);
