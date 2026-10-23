-- An INTEGER has room for any output index: a block has room for some 111,000
-- outputs.
ALTER TABLE brz_ssp_chain_tx_outputs ALTER COLUMN output_index TYPE INTEGER;
ALTER TABLE brz_ssp_chain_tx_inputs
    ALTER COLUMN output_index TYPE INTEGER,
    ALTER COLUMN spending_input_index TYPE INTEGER;
ALTER TABLE brz_ssp_onchain_wallet_spends ALTER COLUMN output_index TYPE INTEGER;
ALTER TABLE brz_ssp_static_deposit_instant_quotes ALTER COLUMN vout TYPE INTEGER;
ALTER TABLE brz_ssp_static_deposit_claims ALTER COLUMN vout TYPE INTEGER;
