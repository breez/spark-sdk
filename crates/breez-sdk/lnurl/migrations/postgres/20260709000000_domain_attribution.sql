-- Per-domain Spark partner attribution, colocated on the allowlist row:
--   api_key: the Breez API key (admin-set), exchanged for a partner JWT.
--   jwt:     the cached partner JWT (server-written), so restarts and sibling
--            instances start warm instead of re-fetching every token.
ALTER TABLE allowed_domains ADD COLUMN api_key TEXT;
ALTER TABLE allowed_domains ADD COLUMN jwt TEXT;
