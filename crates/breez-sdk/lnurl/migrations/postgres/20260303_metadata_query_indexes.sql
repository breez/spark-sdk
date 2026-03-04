CREATE INDEX idx_invoices_user_pubkey_updated_at ON invoices(user_pubkey, updated_at);
CREATE INDEX idx_zaps_user_pubkey_updated_at ON zaps(user_pubkey, updated_at);
CREATE INDEX idx_sender_comments_user_pubkey_updated_at ON sender_comments(user_pubkey, updated_at);
