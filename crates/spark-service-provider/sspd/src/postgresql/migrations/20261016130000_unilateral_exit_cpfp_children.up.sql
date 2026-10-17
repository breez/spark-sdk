-- The CPFP child with which the SSP pays the fee of each transaction it
-- broadcasts to exit a leaf unilaterally. The SSP broadcasts the stored CPFP
-- child every time, so the wallet outputs that are its inputs stay the same
-- until the SSP raises the fee.
CREATE TABLE brz_ssp_unilateral_exit_cpfp_children (
    parent_tx_id TEXT PRIMARY KEY,
    tx_id        TEXT NOT NULL,
    raw          BYTEA NOT NULL
);
