CREATE TABLE brz_ssp_instant_static_deposit_quotes (
    id                   TEXT PRIMARY KEY,
    txid                 TEXT NOT NULL,
    vout                 BIGINT NOT NULL,
    network              TEXT NOT NULL,
    deposit_amount_sats  BIGINT NOT NULL,
    credit_amount_sats   BIGINT NOT NULL,
    destination_address  TEXT NOT NULL,
    quote_signature      TEXT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX brz_ssp_instant_static_deposit_quotes_created_at_idx
    ON brz_ssp_instant_static_deposit_quotes(created_at);

ALTER TABLE brz_ssp_static_deposit_claims
    ADD COLUMN is_instant                   BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN utxo_swap_id                 TEXT,
    ADD COLUMN prep_spend_tx                BYTEA NOT NULL,
    ADD COLUMN prep_spend_nonce_commitments BYTEA NOT NULL,
    ADD COLUMN prep_spend_nonce_ciphertext  BYTEA NOT NULL,
    ADD COLUMN deposit_lost                 BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX brz_ssp_static_deposit_claims_pending_idx
    ON brz_ssp_static_deposit_claims(created_at)
    WHERE pending_transfer_id IS NOT NULL
       OR (transfer_id IS NOT NULL AND NOT spend_confirmed AND NOT deposit_lost);
CREATE INDEX brz_ssp_static_deposit_claims_transfer_idx
    ON brz_ssp_static_deposit_claims(transfer_id);
