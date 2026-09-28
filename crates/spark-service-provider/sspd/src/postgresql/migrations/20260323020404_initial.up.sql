CREATE TABLE brz_ssp_blocks (
    block_hash VARCHAR PRIMARY KEY,
    height BIGINT NOT NULL
);

CREATE INDEX brz_ssp_blocks_height_idx ON brz_ssp_blocks(height);

CREATE TABLE brz_ssp_watch_addresses (
    address VARCHAR PRIMARY KEY
);

CREATE TABLE brz_ssp_tx_outputs (
    tx_id VARCHAR NOT NULL,
    output_index BIGINT NOT NULL,
    address VARCHAR NOT NULL,
    amount BIGINT NOT NULL,
    PRIMARY KEY (tx_id, output_index)
);

CREATE INDEX brz_ssp_tx_outputs_address_idx ON brz_ssp_tx_outputs(address);

-- Deleting a block cascades to its rows here but not to outputs or inputs, so
-- those count as confirmed only while their transaction has a row here.
CREATE TABLE brz_ssp_tx_blocks (
    tx_id VARCHAR NOT NULL,
    block_hash VARCHAR NOT NULL,
    PRIMARY KEY (tx_id, block_hash),
    FOREIGN KEY (block_hash) REFERENCES brz_ssp_blocks (block_hash) ON DELETE CASCADE
);

CREATE INDEX brz_ssp_tx_blocks_block_hash_idx ON brz_ssp_tx_blocks(block_hash);

CREATE TABLE brz_ssp_tx_inputs (
    tx_id VARCHAR NOT NULL,
    output_index BIGINT NOT NULL,
    spending_tx_id VARCHAR NOT NULL,
    spending_input_index BIGINT NOT NULL,
    PRIMARY KEY (spending_tx_id, spending_input_index),
    FOREIGN KEY (tx_id, output_index) REFERENCES brz_ssp_tx_outputs (tx_id, output_index) ON DELETE CASCADE
);

CREATE INDEX brz_ssp_tx_inputs_tx_id_output_index_idx ON brz_ssp_tx_inputs(tx_id, output_index);

