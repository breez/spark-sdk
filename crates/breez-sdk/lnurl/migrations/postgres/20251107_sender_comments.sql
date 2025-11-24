CREATE TABLE sender_comments
(
    payment_hash   VARCHAR(64) NOT NULL PRIMARY KEY,
    user_pubkey    VARCHAR(66) NOT NULL,
    sender_comment VARCHAR(255) NOT NULL,
    updated_at     BIGINT NOT NULL
);

CREATE INDEX idx_sender_comments_user_pubkey ON sender_comments(user_pubkey);
CREATE INDEX idx_sender_comments_updated_at ON sender_comments(updated_at);

ALTER TABLE zaps ADD COLUMN updated_at BIGINT NOT NULL DEFAULT round(extract(epoch from now()) * 1000);
CREATE INDEX idx_zaps_updated_at ON zaps(updated_at);

ALTER TABLE users
ADD COLUMN nostr_pubkey VARCHAR(66);
