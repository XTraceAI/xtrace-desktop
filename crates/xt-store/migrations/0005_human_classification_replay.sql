-- Human classification is now derived before content retention. Unchanged
-- Claude files resume behind checkpoints written before that derivation, so
-- prove them again once: the next native scan replays each transcript through
-- the writer, which may only enrich unknown facts. Reader-host checkpoints,
-- records, receipts and coverage are untouched.
DELETE FROM native_checkpoints WHERE source='transcript';
