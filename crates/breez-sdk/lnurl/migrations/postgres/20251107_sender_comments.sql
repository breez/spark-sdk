CREATE TABLE sender_comments
(
    payment_hash   VARCHAR(64) NOT NULL PRIMARY KEY,
    user_pubkey    VARCHAR(66) NOT NULL,
    sender_comment VARCHAR(255) NOT NULL,
    invoice_expiry BIGINT NOT NULL
);

CREATE INDEX idx_sender_comments_user_pubkey ON sender_comments(user_pubkey);
CREATE INDEX idx_sender_comments_invoice_expiry ON sender_comments(invoice_expiry);

ALTER TABLE users
ADD COLUMN nostr_pubkey VARCHAR(66);