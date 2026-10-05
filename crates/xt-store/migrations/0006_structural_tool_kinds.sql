-- Tool calls now carry their structural kind/server/tool/skill, and Claude
-- native scans now keep the stop-hook summaries they used to discard. Unchanged
-- Claude files resume behind checkpoints written before either, so prove them
-- again once: the next native scan replays each transcript through the writer,
-- which may only fill facts that are still unknown. Reader-host checkpoints,
-- records, receipts, coverage and the migration-5 human classification are
-- untouched, and a checkpoint recreated after this upgrade is never reset again.
DELETE FROM native_checkpoints WHERE source='transcript';
