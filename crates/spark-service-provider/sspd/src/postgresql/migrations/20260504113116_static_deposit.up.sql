CREATE TABLE brz_ssp_static_deposit_claims (
    id                           TEXT PRIMARY KEY,
    user_identity_public_key     BYTEA NOT NULL,
    txid                         TEXT NOT NULL,
    vout                         BIGINT NOT NULL,
    network                      TEXT NOT NULL,
    deposit_address              TEXT NOT NULL,
    credit_amount_sats           BIGINT NOT NULL,
    deposit_amount_sats          BIGINT NOT NULL,
    encrypted_deposit_secret_key TEXT NOT NULL,
    quote_signature              TEXT NOT NULL,
    user_signature               TEXT NOT NULL,
    transfer_id                  TEXT,
    pending_transfer_id          TEXT,
    reservation_id               TEXT,
    reserved_leaf_ids            TEXT[],
    spend_broadcast_txid         TEXT,
    spend_confirmed              BOOLEAN NOT NULL DEFAULT FALSE,
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX brz_ssp_static_deposit_claims_utxo_idx
    ON brz_ssp_static_deposit_claims(txid, vout);
