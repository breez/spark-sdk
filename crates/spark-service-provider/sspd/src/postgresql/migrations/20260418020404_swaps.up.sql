CREATE TABLE brz_ssp_swaps (
    id                       TEXT PRIMARY KEY,
    user_identity_public_key BYTEA NOT NULL,
    -- A transfer is paid for once, even by concurrent requests.
    user_transfer_id         TEXT NOT NULL UNIQUE,
    counter_transfer_id      TEXT NOT NULL,
    reservation_id           TEXT NOT NULL,
    total_amount_sats        BIGINT NOT NULL,
    target_amount_sats       BIGINT NOT NULL,
    fee_sats                 BIGINT NOT NULL,
    -- Set once the SSP holds the user's leaves, even when the claim found none left.
    claimed                  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_swaps_unclaimed_idx ON brz_ssp_swaps(created_at) WHERE NOT claimed;

-- An 'outbound' row is a leaf the counter transfer sends the user. An 'inbound'
-- row is a leaf of the user's, recorded once the SSP has claimed it.
CREATE TABLE brz_ssp_swap_leaves (
    swap_id    TEXT NOT NULL REFERENCES brz_ssp_swaps(id) ON DELETE CASCADE,
    direction  TEXT NOT NULL,
    leaf_id    TEXT NOT NULL,
    value_sats BIGINT NOT NULL,
    PRIMARY KEY (swap_id, direction, leaf_id)
);
