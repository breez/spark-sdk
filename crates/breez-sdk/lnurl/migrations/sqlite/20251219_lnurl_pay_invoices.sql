CREATE TABLE lnurl_pay_invoices
(
    payment_hash         VARCHAR(64) NOT NULL PRIMARY KEY,
    user_pubkey          VARCHAR(66) NOT NULL,
    domain               VARCHAR(255) NOT NULL,
    username             VARCHAR(64) NOT NULL,
    metadata             TEXT NOT NULL,
    invoice_expiry       BIGINT NOT NULL,
    updated_at           BIGINT NOT NULL,
    lightning_receive_id VARCHAR(255),
    bolt11_invoice       TEXT,
    preimage             VARCHAR(64),
    is_privacy_mode      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_lnurl_pay_invoices_user_pubkey ON lnurl_pay_invoices(user_pubkey);
CREATE INDEX idx_lnurl_pay_invoices_invoice_expiry ON lnurl_pay_invoices(invoice_expiry);
CREATE INDEX idx_lnurl_pay_invoices_monitor ON lnurl_pay_invoices(user_pubkey, invoice_expiry, is_privacy_mode) WHERE preimage IS NULL;
