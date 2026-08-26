-- One row per successful registration that changed the name a pubkey holds in
-- a domain, counted to bound how fast one client can churn through addresses.
-- Rows older than the counting window are pruned by the cleanup processor.
CREATE TABLE address_registrations (
	id BIGSERIAL PRIMARY KEY,
	domain VARCHAR(255) NOT NULL,
	pubkey VARCHAR(66) NOT NULL,
	created_at BIGINT NOT NULL
);

CREATE INDEX idx_address_registrations_domain_pubkey_created
	ON address_registrations (domain, pubkey, created_at);

-- The pruner filters on created_at alone, which the index above cannot serve.
CREATE INDEX idx_address_registrations_created_at
	ON address_registrations (created_at);
