-- Claimed leaves wait here until they are fit to join the pool: a leaf whose
-- refund timelock is near the floor is renewed first. A row lost here is a
-- claimed leaf nothing looks for again.
CREATE TABLE brz_ssp_incoming_leaves (
    leaf_id TEXT PRIMARY KEY,
    leaf JSONB NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts INTEGER NOT NULL DEFAULT 0
);

-- Leaves that have failed more are taken later, so a leaf that keeps failing does
-- not hold newer ones up.
CREATE INDEX brz_ssp_incoming_leaves_attempts_claimed_at_idx
    ON brz_ssp_incoming_leaves (attempts, claimed_at);
