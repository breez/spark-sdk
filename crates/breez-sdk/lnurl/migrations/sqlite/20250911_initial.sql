CREATE TABLE users(
	domain VARCHAR(255) NOT NULL,
	pubkey VARCHAR(66) NOT NULL,
	name VARCHAR(64) NOT NULL,
	description VARCHAR(255) NOT NULL,
	PRIMARY KEY (domain, pubkey),
	UNIQUE(domain, name)
);
