-- Add domain column to invoices so we know which domain created the invoice.
ALTER TABLE invoices ADD COLUMN domain VARCHAR(255);

-- Amount received in satoshis (from the HTLC). NULL when unknown.
ALTER TABLE invoices ADD COLUMN amount_received_sat BIGINT;
