-- Relative native order survives copied prefixes whose absolute line offsets differ.
CREATE TABLE native_response_order (
    before_uuid TEXT NOT NULL REFERENCES records(uuid),
    after_uuid TEXT NOT NULL REFERENCES records(uuid),
    PRIMARY KEY(before_uuid,after_uuid),
    CHECK(before_uuid<>after_uuid)
);
CREATE INDEX native_response_order_after ON native_response_order(after_uuid);
-- Per-file continuation state; reset on a full reread, retain on proven append.
CREATE TABLE native_response_heads (
    source_key TEXT NOT NULL,
    api_message_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    record_uuid TEXT NOT NULL REFERENCES records(uuid),
    PRIMARY KEY(source_key,api_message_id,request_id)
);
-- Existing files must supply ordering evidence before unchanged scans can skip them.
DELETE FROM native_checkpoints WHERE source='transcript';
