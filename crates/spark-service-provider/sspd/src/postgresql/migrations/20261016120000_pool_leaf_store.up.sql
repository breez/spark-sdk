-- The SSP reserves a transfer's leaves under the transfer's own id, which each
-- of these tables already has.
ALTER TABLE brz_ssp_swaps DROP COLUMN reservation_id;
ALTER TABLE brz_ssp_lightning_receive_requests
    DROP COLUMN reservation_id,
    DROP COLUMN reserved_leaf_ids;
ALTER TABLE brz_ssp_static_deposit_claims
    DROP COLUMN reservation_id,
    DROP COLUMN reserved_leaf_ids;

-- The SSP stores a claimed leaf as its own when it claims the leaf, so it has
-- no use for this table.
DROP TABLE brz_ssp_incoming_leaves;
