-- The static deposit addresses the coordinator has confirmed to the SSP. An
-- address's user and keys are fixed, so the SSP does not query the coordinator
-- again for an address it has recorded.
CREATE TABLE brz_ssp_static_deposit_addresses (
    address                  TEXT PRIMARY KEY,
    user_identity_public_key BYTEA NOT NULL,
    user_signing_public_key  BYTEA NOT NULL,
    verifying_public_key     BYTEA NOT NULL
);
