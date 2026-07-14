-- Per-domain Spark partner attribution, in a dedicated table indexed by domain:
--   api_key: the Breez API key (admin-set), exchanged for a partner JWT.
--   jwt:     the cached partner JWT (server-written), so restarts and sibling
--            instances start warm instead of re-fetching every token.
CREATE TABLE domain_attribution (
    domain VARCHAR(255) PRIMARY KEY,
    api_key TEXT NOT NULL,
    jwt TEXT
);
