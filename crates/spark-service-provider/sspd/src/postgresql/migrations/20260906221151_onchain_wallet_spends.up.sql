-- Wallet coins withheld from coin selection because the SSP transaction
-- spending_txid spends them. A coin has a row for each fee bump child that
-- spends it.
CREATE TABLE brz_ssp_wallet_spends (
    tx_id         TEXT NOT NULL,
    output_index  BIGINT NOT NULL,
    spending_txid TEXT NOT NULL,
    PRIMARY KEY (tx_id, output_index, spending_txid)
);

CREATE INDEX brz_ssp_wallet_spends_spending_txid_idx ON brz_ssp_wallet_spends(spending_txid);
