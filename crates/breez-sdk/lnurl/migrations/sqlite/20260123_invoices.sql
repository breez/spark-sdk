CREATE TABLE invoices (
    payment_hash TEXT PRIMARY KEY,
    user_pubkey TEXT NOT NULL,
    invoice TEXT NOT NULL,
    preimage TEXT,
    invoice_expiry BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX idx_invoices_user_pubkey ON invoices(user_pubkey);
CREATE INDEX idx_invoices_invoice_expiry ON invoices(invoice_expiry);
CREATE INDEX idx_invoices_updated_at ON invoices(updated_at);

CREATE TABLE newly_paid (
    payment_hash TEXT PRIMARY KEY,
    created_at BIGINT NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at BIGINT NOT NULL
);
CREATE INDEX idx_newly_paid_next_retry_at ON newly_paid(next_retry_at);

ALTER TABLE users ADD COLUMN no_invoice_paid_support INTEGER NOT NULL DEFAULT 0;
