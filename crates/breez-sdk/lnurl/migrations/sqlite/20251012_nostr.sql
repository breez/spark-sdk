ALTER TABLE users ADD COLUMN nostr_pubkey VARCHAR(66) NULL;

CREATE TABLE nostr_zap_requests (
	user_pubkey VARCHAR(66) NOT NULL,
	invoice VARCHAR NOT NULL,
	zap_request VARCHAR NOT NULL,
	created_at INTEGER NOT NULL
);

CREATE TABLE lnurl_sender_comments (
	user_pubkey VARCHAR(66) NOT NULL,
	invoice VARCHAR NOT NULL,
	comment VARCHAR NOT NULL,
	created_at INTEGER NOT NULL
);
