ALTER TABLE brz_ssp_static_deposit_claims
    ADD COLUMN verifying_public_key    BYTEA,
    ADD COLUMN spend_tx_signing_result BYTEA;
