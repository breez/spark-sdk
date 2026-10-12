ALTER TABLE brz_ssp_static_deposit_claims
    ADD COLUMN signed_spend_transaction BYTEA,
    DROP COLUMN prep_spend_nonce_commitments,
    DROP COLUMN prep_spend_nonce_ciphertext,
    DROP COLUMN verifying_public_key,
    DROP COLUMN spend_tx_signing_result;
