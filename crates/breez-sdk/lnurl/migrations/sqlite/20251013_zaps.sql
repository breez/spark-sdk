CREATE TABLE zaps
(
    payment_hash   VARCHAR(64) NOT NULL PRIMARY KEY,
    zap_request    TEXT        NOT NULL,
    zap_event      TEXT,
    user_pubkey    VARCHAR(66) NOT NULL,
    invoice_expiry BIGINT      NOT NULL
);

CREATE INDEX idx_zaps_user_pubkey ON zaps(user_pubkey);
CREATE INDEX idx_zaps_invoice_expiry ON zaps(invoice_expiry);