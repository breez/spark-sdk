CREATE TABLE brz_ssp_coop_exit_requests (
    id                       TEXT PRIMARY KEY,
    user_identity_public_key BYTEA NOT NULL,
    -- A transfer is paid for once, even by concurrent requests.
    user_transfer_id         TEXT NOT NULL UNIQUE,
    withdrawal_address       TEXT NOT NULL,
    amount_sats              BIGINT NOT NULL,
    fee_sats                 BIGINT NOT NULL,
    raw_coop_exit_tx         BYTEA NOT NULL,
    raw_connector_tx         BYTEA NOT NULL,
    coop_exit_txid           TEXT NOT NULL,
    -- A JSON array with one entry per exit tx input.
    prevouts                 TEXT NOT NULL,
    completed                BOOLEAN NOT NULL DEFAULT FALSE,
    broadcast_txid           TEXT,
    leaves_claimed           BOOLEAN NOT NULL DEFAULT FALSE,
    settled                  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_coop_exit_requests_incomplete_idx
    ON brz_ssp_coop_exit_requests(created_at) WHERE NOT completed;
CREATE INDEX brz_ssp_coop_exit_requests_pending_idx
    ON brz_ssp_coop_exit_requests(created_at) WHERE completed AND NOT settled;

-- The leaves an incomplete request names, so no two open requests hold coins
-- against the same leaf.
CREATE TABLE brz_ssp_coop_exit_leaves (
    leaf_id    TEXT PRIMARY KEY,
    request_id TEXT NOT NULL REFERENCES brz_ssp_coop_exit_requests(id) ON DELETE CASCADE
);

CREATE INDEX brz_ssp_coop_exit_leaves_request_id_idx ON brz_ssp_coop_exit_leaves(request_id);
