-- Names each table after the part of the SSP that owns it: the chain record
-- (chain), the on-chain wallet (onchain_wallet), the Spark leaves (spark), the
-- pool built from them (spark_pool) and the static deposits (static_deposit).
-- Postgres renames a primary key constraint together with its index.

ALTER TABLE brz_ssp_blocks RENAME TO brz_ssp_chain_blocks;
ALTER INDEX brz_ssp_blocks_pkey RENAME TO brz_ssp_chain_blocks_pkey;
ALTER INDEX brz_ssp_blocks_height_idx RENAME TO brz_ssp_chain_blocks_height_idx;

ALTER TABLE brz_ssp_watch_addresses RENAME TO brz_ssp_chain_watch_addresses;
ALTER INDEX brz_ssp_watch_addresses_pkey RENAME TO brz_ssp_chain_watch_addresses_pkey;

ALTER TABLE brz_ssp_tx_outputs RENAME TO brz_ssp_chain_tx_outputs;
ALTER INDEX brz_ssp_tx_outputs_pkey RENAME TO brz_ssp_chain_tx_outputs_pkey;
ALTER INDEX brz_ssp_tx_outputs_address_idx RENAME TO brz_ssp_chain_tx_outputs_address_idx;

ALTER TABLE brz_ssp_tx_inputs RENAME TO brz_ssp_chain_tx_inputs;
ALTER INDEX brz_ssp_tx_inputs_pkey RENAME TO brz_ssp_chain_tx_inputs_pkey;
ALTER INDEX brz_ssp_tx_inputs_tx_id_output_index_idx
    RENAME TO brz_ssp_chain_tx_inputs_tx_id_output_index_idx;
ALTER TABLE brz_ssp_chain_tx_inputs
    RENAME CONSTRAINT brz_ssp_tx_inputs_tx_id_fkey TO brz_ssp_chain_tx_inputs_tx_id_fkey;

ALTER TABLE brz_ssp_tx_blocks RENAME TO brz_ssp_chain_tx_blocks;
ALTER INDEX brz_ssp_tx_blocks_pkey RENAME TO brz_ssp_chain_tx_blocks_pkey;
ALTER INDEX brz_ssp_tx_blocks_block_hash_idx RENAME TO brz_ssp_chain_tx_blocks_block_hash_idx;
ALTER TABLE brz_ssp_chain_tx_blocks
    RENAME CONSTRAINT brz_ssp_tx_blocks_block_hash_fkey TO brz_ssp_chain_tx_blocks_block_hash_fkey;

ALTER TABLE brz_ssp_wallet_spends RENAME TO brz_ssp_onchain_wallet_spends;
ALTER INDEX brz_ssp_wallet_spends_pkey RENAME TO brz_ssp_onchain_wallet_spends_pkey;
ALTER INDEX brz_ssp_wallet_spends_spending_txid_idx
    RENAME TO brz_ssp_onchain_wallet_spends_spending_txid_idx;

ALTER TABLE brz_ssp_deposit_txs RENAME TO brz_ssp_spark_pool_deposit_txs;
ALTER INDEX brz_ssp_deposit_txs_pkey RENAME TO brz_ssp_spark_pool_deposit_txs_pkey;
ALTER INDEX brz_ssp_deposit_txs_bump_pending_idx
    RENAME TO brz_ssp_spark_pool_deposit_txs_bump_pending_idx;

ALTER TABLE brz_ssp_deposit_trees RENAME TO brz_ssp_spark_pool_deposit_trees;
ALTER INDEX brz_ssp_deposit_trees_pkey RENAME TO brz_ssp_spark_pool_deposit_trees_pkey;
ALTER INDEX brz_ssp_deposit_trees_unpooled_idx
    RENAME TO brz_ssp_spark_pool_deposit_trees_unpooled_idx;
ALTER TABLE brz_ssp_spark_pool_deposit_trees
    RENAME CONSTRAINT brz_ssp_deposit_trees_txid_fkey TO brz_ssp_spark_pool_deposit_trees_txid_fkey;

ALTER TABLE brz_ssp_funding_bumps RENAME TO brz_ssp_spark_pool_funding_bumps;
ALTER INDEX brz_ssp_funding_bumps_pkey RENAME TO brz_ssp_spark_pool_funding_bumps_pkey;
ALTER INDEX brz_ssp_funding_bumps_funding_txid_idx
    RENAME TO brz_ssp_spark_pool_funding_bumps_funding_txid_idx;
ALTER TABLE brz_ssp_spark_pool_funding_bumps
    RENAME CONSTRAINT brz_ssp_funding_bumps_funding_txid_fkey
    TO brz_ssp_spark_pool_funding_bumps_funding_txid_fkey;

ALTER TABLE brz_ssp_leaf_id_map RENAME TO brz_ssp_spark_leaf_id_map;
ALTER INDEX brz_ssp_leaf_id_map_pkey RENAME TO brz_ssp_spark_leaf_id_map_pkey;

ALTER TABLE brz_ssp_instant_static_deposit_quotes RENAME TO brz_ssp_static_deposit_instant_quotes;
ALTER INDEX brz_ssp_instant_static_deposit_quotes_pkey
    RENAME TO brz_ssp_static_deposit_instant_quotes_pkey;
ALTER INDEX brz_ssp_instant_static_deposit_quotes_created_at_idx
    RENAME TO brz_ssp_static_deposit_instant_quotes_created_at_idx;
