CREATE TABLE zaps
(
    payment_hash VARCHAR(64) NOT NULL PRIMARY KEY,
    zap_request  TEXT        NOT NULL,
    zap_event    TEXT,
);