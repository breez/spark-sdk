-- The id each tree-deposit leaf's signing key derives from, which the SSP picks
-- before the operators assign the leaf's node id. A row is deleted once its leaf
-- signs under its node id.
CREATE TABLE brz_ssp_leaf_id_map (
    node_id         TEXT PRIMARY KEY,
    deposit_leaf_id TEXT NOT NULL
);
