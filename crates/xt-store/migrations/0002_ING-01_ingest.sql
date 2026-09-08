-- Extend canonical rows without duplicating source, surface or usage columns.
ALTER TABLE records ADD COLUMN parent_uuid TEXT;
ALTER TABLE records ADD COLUMN agent_id TEXT;
ALTER TABLE records ADD COLUMN subtype TEXT;
ALTER TABLE records ADD COLUMN first_seen_at INTEGER;
ALTER TABLE records ADD COLUMN is_human INTEGER CHECK(is_human IN (0, 1));
ALTER TABLE records ADD COLUMN is_command INTEGER CHECK(is_command IN (0, 1));
ALTER TABLE records ADD COLUMN is_interrupted INTEGER CHECK(is_interrupted IN (0, 1));
ALTER TABLE records ADD COLUMN is_system_reminder INTEGER CHECK(is_system_reminder IN (0, 1));
ALTER TABLE sessions ADD COLUMN repo TEXT;
ALTER TABLE sessions ADD COLUMN namespace TEXT;
ALTER TABLE sessions ADD COLUMN git_branches TEXT CHECK(git_branches IS NULL OR (json_valid(git_branches) AND json_type(git_branches)='array'));
ALTER TABLE sessions ADD COLUMN record_count INTEGER NOT NULL DEFAULT 0 CHECK(record_count >= 0);
ALTER TABLE sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'user' CHECK(kind IN ('user', 'judge'));
UPDATE sessions SET record_count=(SELECT count(*) FROM records WHERE records.session_id=sessions.session_id);
CREATE UNIQUE INDEX records_uuid_session ON records(uuid, session_id);

CREATE TABLE hosts (
    host TEXT PRIMARY KEY NOT NULL CHECK(host IN ('claude', 'codex', 'cursor', 'other')),
    installed INTEGER CHECK(installed IN (0, 1)),
    version TEXT,
    last_probed_at INTEGER
);
CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL CHECK(length(trim(key)) > 0),
    value_json TEXT NOT NULL CHECK(json_valid(value_json))
);
CREATE TABLE source_cursors (
    source TEXT NOT NULL CHECK(source IN ('plugin', 'transcript', 'readers_cli', 'fixture')),
    cursor_key TEXT NOT NULL CHECK(length(trim(cursor_key)) > 0),
    position INTEGER NOT NULL CHECK(position >= 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(source, cursor_key)
);
CREATE TABLE session_sources (
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    source TEXT NOT NULL CHECK(source IN ('plugin', 'transcript', 'readers_cli', 'fixture')),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL CHECK(last_seen_at >= first_seen_at),
    PRIMARY KEY(session_id, source)
);
CREATE TABLE record_sources (
    uuid TEXT NOT NULL REFERENCES records(uuid),
    source TEXT NOT NULL CHECK(source IN ('plugin', 'transcript', 'readers_cli', 'fixture')),
    field_presence INTEGER NOT NULL CHECK(typeof(field_presence)='integer' AND field_presence >= 0),
    conflict_flags INTEGER NOT NULL CHECK(typeof(conflict_flags)='integer' AND conflict_flags >= 0),
    PRIMARY KEY(uuid, source)
);

CREATE TABLE capture_receipts (
    receipt_id TEXT PRIMARY KEY NOT NULL CHECK(length(trim(receipt_id)) > 0),
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    surface TEXT,
    received_at INTEGER NOT NULL,
    coverage_sealed INTEGER NOT NULL DEFAULT 0 CHECK(coverage_sealed IN (0, 1)),
    UNIQUE(receipt_id, session_id)
);
CREATE TABLE capture_record_coverage (
    receipt_id TEXT NOT NULL,
    record_uuid TEXT NOT NULL,
    session_id TEXT NOT NULL,
    metric_field_mask INTEGER NOT NULL CHECK(typeof(metric_field_mask)='integer' AND metric_field_mask >= 0),
    measurement_revision TEXT NOT NULL CHECK(typeof(measurement_revision)='text' AND length(measurement_revision)=64 AND measurement_revision NOT GLOB '*[^0-9a-f]*'),
    digest_schema_version INTEGER NOT NULL CHECK(typeof(digest_schema_version)='integer' AND digest_schema_version > 0),
    PRIMARY KEY(receipt_id, record_uuid),
    FOREIGN KEY(receipt_id, session_id) REFERENCES capture_receipts(receipt_id, session_id),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
-- Seal the complete submitted set in the same transaction as its insertion.
-- Later enrichment cannot add coverage to, edit or delete an earlier receipt.
CREATE TRIGGER capture_receipts_unsealed_insert BEFORE INSERT ON capture_receipts
WHEN NEW.coverage_sealed != 0 OR EXISTS(SELECT 1 FROM capture_receipts WHERE receipt_id=NEW.receipt_id)
BEGIN SELECT RAISE(ABORT, 'capture receipt must be inserted unsealed'); END;
CREATE TRIGGER capture_receipts_immutable_update BEFORE UPDATE ON capture_receipts
WHEN NOT (OLD.coverage_sealed=0 AND NEW.coverage_sealed=1
    AND NEW.receipt_id IS OLD.receipt_id AND NEW.session_id IS OLD.session_id
    AND NEW.surface IS OLD.surface AND NEW.received_at IS OLD.received_at
    AND EXISTS(SELECT 1 FROM capture_record_coverage WHERE receipt_id=OLD.receipt_id))
BEGIN SELECT RAISE(ABORT, 'capture receipt is immutable'); END;
CREATE TRIGGER capture_receipts_immutable_delete BEFORE DELETE ON capture_receipts
BEGIN SELECT RAISE(ABORT, 'capture receipt is immutable'); END;
CREATE TRIGGER capture_coverage_sealed_insert BEFORE INSERT ON capture_record_coverage
WHEN coalesce((SELECT coverage_sealed FROM capture_receipts WHERE receipt_id=NEW.receipt_id), 1) != 0
BEGIN SELECT RAISE(ABORT, 'capture coverage is sealed'); END;
CREATE TRIGGER capture_coverage_immutable_update BEFORE UPDATE ON capture_record_coverage
BEGIN SELECT RAISE(ABORT, 'capture coverage is immutable'); END;
CREATE TRIGGER capture_coverage_immutable_delete BEFORE DELETE ON capture_record_coverage
BEGIN SELECT RAISE(ABORT, 'capture coverage is immutable'); END;
CREATE INDEX capture_receipts_session ON capture_receipts(session_id, received_at);
CREATE INDEX capture_coverage_record ON capture_record_coverage(record_uuid);

-- Discovery can precede canonical ingestion, so conversation_id is not a FK.
CREATE TABLE discovered_sessions (
    host TEXT NOT NULL CHECK(host IN ('claude', 'codex', 'cursor', 'other')),
    native_session_id TEXT NOT NULL CHECK(length(trim(native_session_id)) > 0),
    conversation_id TEXT CHECK(conversation_id IS NULL OR length(trim(conversation_id)) > 0),
    surface TEXT,
    started_at_ms INTEGER,
    last_observed_at INTEGER NOT NULL,
    discovery_complete INTEGER NOT NULL CHECK(discovery_complete IN (0, 1)),
    PRIMARY KEY(host, native_session_id)
);
CREATE INDEX discovered_sessions_surface_start ON discovered_sessions(host, surface, started_at_ms);

-- SQLite cannot relax the original uuid NOT NULL with ALTER COLUMN. Rebuild
-- only this table, preserving every existing ID, block index and content value.
CREATE TABLE tool_uses_ingest (
    id INTEGER PRIMARY KEY,
    uuid TEXT,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    block_index INTEGER CHECK(block_index >= 0),
    name TEXT NOT NULL,
    input_json TEXT CHECK(input_json IS NULL OR json_valid(input_json)),
    kind TEXT CHECK(kind IN ('builtin', 'mcp', 'skill', 'hook', 'command', 'subagent')),
    server TEXT,
    tool TEXT,
    skill TEXT,
    source TEXT CHECK(source IN ('plugin', 'transcript', 'readers_cli', 'fixture')),
    source_event_id TEXT CHECK(source_event_id IS NULL OR length(trim(source_event_id)) > 0),
    event_ts TEXT,
    UNIQUE(uuid, block_index),
    UNIQUE(session_id, source, source_event_id),
    FOREIGN KEY(uuid, session_id) REFERENCES records(uuid, session_id),
    CHECK((uuid IS NULL) = (block_index IS NULL)),
    CHECK(uuid IS NOT NULL OR (source IS NOT NULL AND source_event_id IS NOT NULL)),
    CHECK(uuid IS NOT NULL OR input_json IS NULL),
    CHECK(uuid IS NULL OR event_ts IS NULL)
);
INSERT INTO tool_uses_ingest(id, uuid, session_id, block_index, name, input_json)
SELECT t.id, t.uuid, r.session_id, t.block_index, t.name, t.input_json
FROM tool_uses t LEFT JOIN records r ON r.uuid=t.uuid;
DROP TABLE tool_uses;
ALTER TABLE tool_uses_ingest RENAME TO tool_uses;
CREATE INDEX tool_uses_session_kind ON tool_uses(session_id, kind);

CREATE TABLE pull_requests (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL CHECK(length(trim(repo)) > 0),
    number INTEGER NOT NULL CHECK(number > 0),
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    state TEXT CHECK(state IN ('OPEN', 'CLOSED', 'MERGED')),
    merged_at TEXT,
    additions INTEGER CHECK(additions >= 0),
    deletions INTEGER CHECK(deletions >= 0),
    head_ref_name TEXT,
    refreshed_at INTEGER,
    UNIQUE(repo, number)
);
CREATE TABLE pr_links (
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    pr_id INTEGER NOT NULL REFERENCES pull_requests(id),
    confidence TEXT NOT NULL CHECK(confidence IN ('exact', 'sha', 'inferred')),
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL CHECK(last_seen_at >= first_seen_at),
    PRIMARY KEY(session_id, pr_id)
);
