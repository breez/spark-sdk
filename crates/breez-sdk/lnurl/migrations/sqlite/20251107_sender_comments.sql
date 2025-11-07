CREATE TABLE sender_comments
(
    payment_hash   TEXT NOT NULL PRIMARY KEY,
    user_pubkey    TEXT NOT NULL,
    sender_comment TEXT NOT NULL,
    invoice_expiry BIGINT NOT NULL
);

CREATE INDEX idx_sender_comments_user_pubkey ON sender_comments(user_pubkey);
CREATE INDEX idx_sender_comments_invoice_expiry ON sender_comments(invoice_expiry);
