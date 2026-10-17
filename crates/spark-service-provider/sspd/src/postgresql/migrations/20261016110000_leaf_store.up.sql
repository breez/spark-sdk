-- The SSP works out from these tables and the chain record whether it can use
-- a leaf, each time it needs to know, rather than storing a status.

-- Transactions the SSP has a valid signature for. timelock is the relative
-- timelock in blocks. Among the transactions with the same input, the
-- transaction with the lowest timelock can confirm first.
CREATE TABLE brz_ssp_spark_txs (
    tx_id                 TEXT PRIMARY KEY,
    previous_tx_id        TEXT NOT NULL,
    previous_output_index INTEGER NOT NULL,
    timelock              INTEGER NOT NULL,
    raw                   BYTEA NOT NULL
);

CREATE INDEX brz_ssp_spark_txs_previous_output_idx
    ON brz_ssp_spark_txs (previous_tx_id, previous_output_index, timelock);

-- The operators' direct transactions, each for a cpfp transaction. The SSP need
-- not have a signature for them. It keeps them to build a receiver's refunds
-- from, and to work out what the operators' watchtower can broadcast.
CREATE TABLE brz_ssp_spark_direct_txs (
    tx_id                 TEXT PRIMARY KEY,
    cpfp_tx_id            TEXT NOT NULL REFERENCES brz_ssp_spark_txs (tx_id),
    previous_tx_id        TEXT NOT NULL,
    previous_output_index INTEGER NOT NULL,
    timelock              INTEGER NOT NULL,
    raw                   BYTEA NOT NULL,
    stored_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_spark_direct_txs_cpfp_idx
    ON brz_ssp_spark_direct_txs (cpfp_tx_id, stored_at);

CREATE INDEX brz_ssp_spark_direct_txs_previous_output_idx
    ON brz_ssp_spark_direct_txs (previous_tx_id, previous_output_index);

-- A node id is the operators' name for a node, and is not part of any
-- transaction. keyshare_owners has the owners' identifiers, 32 bytes each.
CREATE TABLE brz_ssp_spark_nodes (
    node_id            UUID PRIMARY KEY,
    tree_id            UUID NOT NULL,
    parent_node_id     UUID,
    node_tx_id         TEXT NOT NULL REFERENCES brz_ssp_spark_txs (tx_id),
    verifying_key      BYTEA NOT NULL,
    keyshare_key       BYTEA NOT NULL,
    keyshare_threshold INTEGER NOT NULL,
    keyshare_owners    BYTEA NOT NULL
);

CREATE INDEX brz_ssp_spark_nodes_node_tx_idx ON brz_ssp_spark_nodes (node_tx_id);

-- The leaves the SSP owns, each with the refund the SSP has for it. A leaf
-- has a new refund after each transfer and each renewal, so the store keys a
-- leaf by its refund. The refund's input is an output of the leaf's node
-- transaction, so a refund has only one exit chain above it. The SSP only
-- inserts and deletes rows. value is the value of the output that is the
-- refund's input, and refund_timelock is the refund's own timelock.
CREATE TABLE brz_ssp_spark_leaves (
    refund_tx_id    TEXT PRIMARY KEY REFERENCES brz_ssp_spark_txs (tx_id),
    node_id         UUID NOT NULL UNIQUE REFERENCES brz_ssp_spark_nodes (node_id),
    value           BIGINT NOT NULL,
    refund_timelock INTEGER NOT NULL
);

-- The order in which the SSP reserves the leaves of one value.
CREATE INDEX brz_ssp_spark_leaves_value_idx
    ON brz_ssp_spark_leaves (value, refund_timelock, node_id);

-- The order in which the SSP reads the leaves whose refund needs renewal.
CREATE INDEX brz_ssp_spark_leaves_refund_timelock_idx
    ON brz_ssp_spark_leaves (refund_timelock, node_id);

CREATE TABLE brz_ssp_spark_reservations (
    refund_tx_id TEXT PRIMARY KEY
        REFERENCES brz_ssp_spark_leaves (refund_tx_id) ON DELETE CASCADE,
    transfer_id  UUID NOT NULL,
    reserved_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_spark_reservations_transfer_idx ON brz_ssp_spark_reservations (transfer_id);

-- What the operators last reported for each leaf of the SSP's. The store keeps
-- their view apart from the SSP's own. The SSP uses a leaf only while the two
-- agree, and a difference that lasts is a fault to investigate.
CREATE TABLE brz_ssp_spark_operator_nodes (
    node_id                   UUID PRIMARY KEY
        REFERENCES brz_ssp_spark_leaves (node_id) ON DELETE CASCADE,
    status                    TEXT NOT NULL,
    owner_identity_public_key BYTEA,
    node_tx_id                TEXT NOT NULL,
    refund_tx_id              TEXT,
    keyshare_key              BYTEA NOT NULL,
    observed_at               TIMESTAMPTZ NOT NULL
);

-- Leaves the SSP transferred. A refund listed here is never again the refund
-- of a leaf the SSP owns.
CREATE TABLE brz_ssp_spark_transferred_leaves (
    refund_tx_id TEXT PRIMARY KEY REFERENCES brz_ssp_spark_txs (tx_id),
    node_id      UUID NOT NULL,
    transfer_id  UUID NOT NULL
);

CREATE INDEX brz_ssp_spark_transferred_leaves_transfer_idx
    ON brz_ssp_spark_transferred_leaves (transfer_id);

-- Transfers whose leaves the SSP has stored. The SSP stores the leaves of a
-- transfer only once, whatever the operators return when it claims the
-- transfer again.
CREATE TABLE brz_ssp_spark_claimed_transfers (
    transfer_id UUID PRIMARY KEY
);
