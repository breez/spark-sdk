-- A name whose owner gave it up. While the row stands, only `pubkey` may
-- register the name again: payers keep paying an address long after its owner
-- drops it, so handing it to a stranger misdirects their payments.
--
-- reclaimable_from is when anyone may register it. Every hold is written NULL,
-- which reserves the name for good; the column is what a policy that lets holds
-- lapse (a per-partner cooldown, say) would fill in.
CREATE TABLE released_names (
	domain VARCHAR(255) NOT NULL,
	name VARCHAR(64) NOT NULL,
	pubkey VARCHAR(66) NOT NULL,
	released_at BIGINT NOT NULL,
	reclaimable_from BIGINT,
	PRIMARY KEY (domain, name)
);
