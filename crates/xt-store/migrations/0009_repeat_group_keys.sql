-- M-20's grouping identity beside each tool call: the derivation's version,
-- its opaque comparison key, and whether two observations of the same call
-- disagreed about it. The key is derived in memory before content retention
-- decides about input_json, so a metadata-only row carries it; see
-- src/repeat_key.rs for what it is and why it may never be exported.
--
-- Rows an older build wrote state no key and stay unknown. They are not
-- backfilled: a stored input_json is a retained copy of content, not a fresh
-- observation, and deriving keys from it would make the answer depend on
-- whether content happened to be kept. Transcript checkpoints are deliberately
-- left alone — unlike migrations 5, 6 and 7 this upgrade asks for no replay, so
-- no scan is re-proven, nothing already imported is re-read, and a stretch
-- whose calls predate this column keeps saying its repeat count is unknown
-- until those records are written again for some other reason.
ALTER TABLE tool_uses ADD COLUMN group_version INTEGER CHECK(
    group_version IS NULL OR (typeof(group_version) = 'integer' AND group_version > 0)
);
ALTER TABLE tool_uses ADD COLUMN group_key TEXT CHECK(
    (group_key IS NULL) = (group_version IS NULL)
    AND (group_key IS NULL OR (length(group_key) = 64 AND group_key NOT GLOB '*[^0-9a-f]*'))
);
ALTER TABLE tool_uses ADD COLUMN group_conflict INTEGER NOT NULL DEFAULT 0 CHECK(
    group_conflict IN (0, 1)
);
