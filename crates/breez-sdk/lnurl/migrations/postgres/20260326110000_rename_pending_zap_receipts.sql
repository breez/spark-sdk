-- This table specifically tracks zap receipt publishing, not generic payment events.
ALTER TABLE newly_paid RENAME TO pending_zap_receipts;
ALTER INDEX idx_newly_paid_next_retry_at RENAME TO idx_pending_zap_receipts_next_retry_at;
