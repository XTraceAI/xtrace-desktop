CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY NOT NULL CHECK(length(trim(session_id)) > 0),
    host TEXT NOT NULL CHECK(host IN ('claude', 'codex', 'cursor', 'other')),
    source_platform TEXT,
    cwd TEXT,
    git_branch TEXT,
    title TEXT,
    surface TEXT,
    surface_evidence_json TEXT CHECK(surface_evidence_json IS NULL OR json_valid(surface_evidence_json)),
    native_session_id TEXT,
    started_at_ms INTEGER,
    first_ts TEXT,
    last_ts TEXT,
    source TEXT NOT NULL CHECK(source IN ('plugin', 'transcript', 'readers_cli', 'fixture')),
    has_conflict INTEGER NOT NULL DEFAULT 0 CHECK(has_conflict IN (0, 1))
);

CREATE TABLE records (
    uuid TEXT PRIMARY KEY NOT NULL CHECK(length(trim(uuid)) > 0),
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    type TEXT NOT NULL CHECK(type IN ('user', 'assistant')),
    ts TEXT,
    ts_ms INTEGER,
    api_message_id TEXT,
    request_id TEXT,
    is_meta INTEGER NOT NULL CHECK(is_meta IN (0, 1)),
    is_sidechain INTEGER NOT NULL CHECK(is_sidechain IN (0, 1)),
    role TEXT,
    model TEXT,
    is_tool_result_carrier INTEGER CHECK(is_tool_result_carrier IN (0, 1)),
    text_len INTEGER CHECK(text_len >= 0),
    tool_use_count INTEGER CHECK(tool_use_count >= 0),
    content_json TEXT CHECK(content_json IS NULL OR json_valid(content_json)),
    has_conflict INTEGER NOT NULL DEFAULT 0 CHECK(has_conflict IN (0, 1)),
    CHECK((ts IS NULL) = (ts_ms IS NULL))
);

CREATE TABLE usage (
    uuid TEXT PRIMARY KEY NOT NULL REFERENCES records(uuid),
    input_tokens INTEGER CHECK(input_tokens >= 0),
    output_tokens INTEGER CHECK(output_tokens >= 0),
    cache_read_tokens INTEGER CHECK(cache_read_tokens >= 0),
    cache_creation_tokens INTEGER CHECK(cache_creation_tokens >= 0),
    cache_creation_5m INTEGER CHECK(cache_creation_5m >= 0),
    cache_creation_1h INTEGER CHECK(cache_creation_1h >= 0),
    service_tier TEXT
);

CREATE TABLE tool_uses (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL REFERENCES records(uuid),
    block_index INTEGER NOT NULL CHECK(block_index >= 0),
    name TEXT NOT NULL,
    input_json TEXT CHECK(input_json IS NULL OR json_valid(input_json)),
    UNIQUE(uuid, block_index)
);

CREATE INDEX records_session_ts ON records(session_id, ts_ms);
CREATE INDEX records_ts ON records(ts_ms);
CREATE INDEX records_response ON records(api_message_id, request_id);
CREATE INDEX sessions_host_first_ts ON sessions(host, first_ts);
CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT);
