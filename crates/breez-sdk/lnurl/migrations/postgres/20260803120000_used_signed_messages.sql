-- Records every signed statement a mutating route has acted on, so the same
-- signature is never acted on twice.
--
-- Keyed on the statement rather than the signature bytes: ECDSA signatures are
-- malleable, so two distinct signatures can authorize the same statement.
CREATE TABLE used_signed_messages (
    statement_hash BYTEA PRIMARY KEY,
    -- Which route acted on the statement. Not part of any decision: a claimed
    -- statement is refused whoever claimed it. Kept for diagnostics.
    route TEXT NOT NULL,
    -- When the row may be pruned, which is the point past which the statement
    -- is refused on its own. A statement nothing else bounds in time is
    -- recorded with an expiry that never arrives.
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX idx_used_signed_messages_expires_at
    ON used_signed_messages(expires_at);
