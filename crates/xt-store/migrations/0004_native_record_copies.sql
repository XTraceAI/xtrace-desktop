-- Native forks can contain the same immutable record in several session files.
-- Keep one work record and record additional contexts without transferring it.
CREATE TABLE native_record_copies (
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    record_uuid TEXT NOT NULL REFERENCES records(uuid),
    parent_uuid TEXT,
    PRIMARY KEY(session_id, record_uuid)
);
CREATE INDEX native_record_copies_record ON native_record_copies(record_uuid);

-- Per-session context may overlap; overall work totals count distinct UUIDs.
CREATE VIEW session_work_records AS
    SELECT session_id,uuid AS record_uuid,parent_uuid FROM records
    UNION ALL
    SELECT session_id,record_uuid,parent_uuid FROM native_record_copies;

-- The native interpretation changed: prove unchanged Claude files again once.
DELETE FROM native_checkpoints WHERE source='transcript';
