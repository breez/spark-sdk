-- Addresses the onchain wallet derived, which tell its coins apart from outputs
-- at other watched addresses.
CREATE TABLE brz_ssp_onchain_wallet_addresses (
    address TEXT PRIMARY KEY,
    derivation_index BIGINT NOT NULL
);
