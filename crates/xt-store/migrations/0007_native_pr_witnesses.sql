-- Claude native scans now persist the exact pr-link witnesses they used to
-- ignore. Unchanged Claude files resume behind checkpoints written before that,
-- so prove them again once: the next native scan replays each transcript
-- through the writer, which links only what a witness states and otherwise may
-- only fill facts that are still unknown. Reader-host checkpoints, sessions,
-- records, tool events, receipts, coverage and pull-request rows are untouched,
-- and a checkpoint recreated after this upgrade is never reset again.
DELETE FROM native_checkpoints WHERE source='transcript';
