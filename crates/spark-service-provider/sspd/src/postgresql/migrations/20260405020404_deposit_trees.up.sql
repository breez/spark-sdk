-- Funding transactions the SSP built for its leaf pool.
CREATE TABLE brz_ssp_deposit_txs (
    txid          TEXT PRIMARY KEY,
    raw_tx        BYTEA NOT NULL,
    fee_sats      BIGINT NOT NULL,
    stored_height BIGINT NOT NULL,
    -- Set while a fee bump child is stored and no child has confirmed deep enough
    -- to rule out the others.
    bump_pending  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_deposit_txs_bump_pending_idx ON brz_ssp_deposit_txs(txid) WHERE bump_pending;

-- The fee bump children built for a funding transaction, all kept while any of
-- them can still confirm.
CREATE TABLE brz_ssp_funding_bumps (
    txid         TEXT PRIMARY KEY,
    funding_txid TEXT NOT NULL REFERENCES brz_ssp_deposit_txs(txid) ON DELETE CASCADE,
    raw_tx       BYTEA NOT NULL,
    fee_sats     BIGINT NOT NULL,
    height       BIGINT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_funding_bumps_funding_txid_idx ON brz_ssp_funding_bumps(funding_txid, created_at);

-- One row per tree, keyed by the funding output its root spends. pooled is set once
-- the tree's leaves are in the leaf store.
CREATE TABLE brz_ssp_deposit_trees (
    txid            TEXT NOT NULL REFERENCES brz_ssp_deposit_txs(txid),
    vout            INT NOT NULL,
    deposit_address TEXT NOT NULL,
    denomination    BIGINT NOT NULL,
    leaf_count      INT NOT NULL,
    leaves          JSONB NOT NULL,
    branches        JSONB NOT NULL,
    pooled          BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (txid, vout)
);

CREATE INDEX brz_ssp_deposit_trees_unpooled_idx ON brz_ssp_deposit_trees(txid) WHERE NOT pooled;
