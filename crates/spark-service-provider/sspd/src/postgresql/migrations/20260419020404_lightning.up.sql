CREATE TABLE brz_ssp_lightning_send_requests (
    id                       TEXT PRIMARY KEY,
    user_identity_public_key BYTEA NOT NULL,
    encoded_invoice          TEXT NOT NULL,
    payment_hash             BYTEA NOT NULL,
    amount_sats              BIGINT NOT NULL,
    fee_sats                 BIGINT NOT NULL,
    user_transfer_id         TEXT NOT NULL,
    htlc_address             TEXT NOT NULL,
    htlc_sweep_address       TEXT,
    idempotency_key          TEXT,
    ln_payment_id            TEXT,
    preimage                 BYTEA,
    payment_status           TEXT NOT NULL DEFAULT 'pending',
    leaves_claimed           BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX brz_ssp_lightning_send_requests_user_transfer_idx
    ON brz_ssp_lightning_send_requests(user_transfer_id);
CREATE INDEX brz_ssp_lightning_send_requests_htlc_address_idx
    ON brz_ssp_lightning_send_requests(htlc_address);
CREATE UNIQUE INDEX brz_ssp_lightning_send_requests_payment_hash_idx
    ON brz_ssp_lightning_send_requests(payment_hash)
    WHERE payment_status <> 'failed';
CREATE UNIQUE INDEX brz_ssp_lightning_send_requests_idempotency_key_idx
    ON brz_ssp_lightning_send_requests(idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX brz_ssp_lightning_send_requests_pending_idx
    ON brz_ssp_lightning_send_requests(created_at)
    WHERE payment_status = 'pending' OR (payment_status = 'succeeded' AND NOT leaves_claimed);

-- user_identity_public_key is the receiver; requester_identity_public_key asked
-- for the invoice.
CREATE TABLE brz_ssp_lightning_receive_requests (
    id                            TEXT PRIMARY KEY,
    user_identity_public_key      BYTEA NOT NULL,
    requester_identity_public_key BYTEA NOT NULL,
    payment_hash                  BYTEA NOT NULL,
    amount_sats                   BIGINT NOT NULL,
    encoded_invoice               TEXT NOT NULL,
    transfer_id                   TEXT,
    transfer_amount_sats          BIGINT,
    reservation_id                TEXT,
    reserved_leaf_ids             TEXT[],
    preimage                      BYTEA,
    invoice_status                TEXT NOT NULL DEFAULT 'pending',
    expires_at                    TIMESTAMPTZ NOT NULL,
    memo                          TEXT,
    created_at                    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX brz_ssp_lightning_receive_requests_payment_hash_idx
    ON brz_ssp_lightning_receive_requests(payment_hash);
CREATE INDEX brz_ssp_lightning_receive_requests_unpaid_idx
    ON brz_ssp_lightning_receive_requests(expires_at)
    WHERE invoice_status = 'pending' AND transfer_id IS NULL;
CREATE INDEX brz_ssp_lightning_receive_requests_handover_idx
    ON brz_ssp_lightning_receive_requests(created_at)
    WHERE invoice_status = 'pending' AND transfer_id IS NOT NULL;
