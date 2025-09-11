 CREATE TABLE IF NOT EXISTS payments (
    id TEXT PRIMARY KEY,
    payment_type TEXT NOT NULL,
    status TEXT NOT NULL,
    amount INTEGER NOT NULL,
    fees INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    details TEXT,
    method TEXT
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS unclaimed_deposits (
    txid TEXT NOT NULL,
    vout INTEGER NOT NULL,
    amount_sats INTEGER,
    claim_error TEXT,
    refund_tx TEXT,
    refund_tx_id TEXT,
    PRIMARY KEY (txid, vout)
);
CREATE TABLE IF NOT EXISTS payment_metadata (
    payment_id TEXT PRIMARY KEY,
    lnurl_pay_info TEXT
);
CREATE TABLE IF NOT EXISTS deposit_refunds (
    deposit_tx_id TEXT NOT NULL,
    deposit_vout INTEGER NOT NULL,
    refund_tx TEXT NOT NULL,
    refund_tx_id TEXT NOT NULL,
    PRIMARY KEY (deposit_tx_id, deposit_vout)              
);