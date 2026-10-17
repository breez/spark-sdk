-- The outputs whose spends sspd records. input_tx_id and input_output_index
-- are the input of the transaction that has the output, or NULL when sspd does
-- not have that transaction. The output is on chain only once that transaction
-- is the confirmed spend of its input.
CREATE TABLE brz_ssp_chain_watch_outputs (
    tx_id              TEXT NOT NULL,
    output_index       INTEGER NOT NULL,
    input_tx_id        TEXT,
    input_output_index INTEGER,
    PRIMARY KEY (tx_id, output_index)
);

CREATE INDEX brz_ssp_chain_watch_outputs_input_idx
    ON brz_ssp_chain_watch_outputs (input_tx_id, input_output_index);

-- The watched outputs that are spent on chain. Postgres deletes a row with its
-- block when the chain monitor undoes the block. A spend in a block older than
-- the chain record has no block_hash, and its row stays.
CREATE TABLE brz_ssp_chain_spent_outputs (
    tx_id          TEXT NOT NULL,
    output_index   INTEGER NOT NULL,
    spending_tx_id TEXT NOT NULL,
    block_hash     TEXT
        REFERENCES brz_ssp_chain_blocks (block_hash) ON DELETE CASCADE,
    height         BIGINT NOT NULL,
    PRIMARY KEY (tx_id, output_index)
);

CREATE INDEX brz_ssp_chain_spent_outputs_block_hash_idx
    ON brz_ssp_chain_spent_outputs (block_hash);

-- The watched outputs that the spend check has yet to check with bitcoind. The
-- chain monitor records the spend of a watched output only from the blocks it
-- reads after sspd started to watch the output, so the spend check queries
-- bitcoind for a spend in an earlier block.
CREATE TABLE brz_ssp_chain_unchecked_outputs (
    tx_id        TEXT NOT NULL,
    output_index INTEGER NOT NULL,
    PRIMARY KEY (tx_id, output_index)
);
