ALTER TABLE brz_ssp_static_deposit_claims
    ALTER COLUMN transfer_id TYPE UUID USING transfer_id::uuid,
    ALTER COLUMN pending_transfer_id TYPE UUID USING pending_transfer_id::uuid;
ALTER TABLE brz_ssp_swaps
    ALTER COLUMN user_transfer_id TYPE UUID USING user_transfer_id::uuid,
    ALTER COLUMN counter_transfer_id TYPE UUID USING counter_transfer_id::uuid;
ALTER TABLE brz_ssp_lightning_send_requests
    ALTER COLUMN user_transfer_id TYPE UUID USING user_transfer_id::uuid;
ALTER TABLE brz_ssp_lightning_receive_requests
    ALTER COLUMN transfer_id TYPE UUID USING transfer_id::uuid;
ALTER TABLE brz_ssp_coop_exit_requests
    ALTER COLUMN user_transfer_id TYPE UUID USING user_transfer_id::uuid;
