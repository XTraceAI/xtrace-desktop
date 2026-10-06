-- Whether the supported checks of who created an indexed session have
-- finished against what the index holds of it now. It feeds the display
-- state of the Sessions and Dashboard reads only: a session whose checks
-- have not finished is not listed as a session of its own until they have.
-- No metric, span, usage, Human input, search, membership or ordering query
-- reads it, and a finished check is not evidence that a person created the
-- session: only that the supported checks finished without child evidence.
--
-- One row per session. `required_generation` is the session's current check
-- attempt: 1 when its first records commit, and moved on only when the
-- inputs of its own checks change (its opening, identity or first input, as
-- `own_check_key` names them), or when a launch naming exactly this session
-- appears or changes. An ordinary later message changes neither.
-- `own_check_key` is a digest of those own-check inputs as last observed:
-- identifiers, times and digests only, never a message body.
-- `completed_generation` is the attempt the checks last finished for, and
-- `detector_versions` the versions of the detectors that ran then, written
-- only if the session still required that attempt with that key.
--
-- Every session indexed before this table existed starts unchecked: its
-- checks run again before it is listed on its own.
CREATE TABLE session_child_checks (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id),
    required_generation INTEGER NOT NULL CHECK(
        typeof(required_generation) = 'integer' AND required_generation > 0),
    completed_generation INTEGER CHECK(completed_generation IS NULL OR (
        typeof(completed_generation) = 'integer' AND completed_generation > 0
        AND completed_generation <= required_generation)),
    detector_versions TEXT CHECK(detector_versions IS NULL OR (
        length(detector_versions) BETWEEN 1 AND 64
        AND detector_versions NOT GLOB '*[^0-9a-z.]*')),
    own_check_key TEXT CHECK(own_check_key IS NULL OR (
        length(own_check_key) BETWEEN 1 AND 96
        AND own_check_key NOT GLOB '*[^0-9a-z:]*')),
    CHECK((completed_generation IS NULL) = (detector_versions IS NULL))
) WITHOUT ROWID;

INSERT INTO session_child_checks(session_id, required_generation)
SELECT session_id, 1 FROM sessions;

-- How many launches a thread's published validation found started with no
-- first result yet. A launch without its result names no child, but may
-- still be the one that created a session born after it, so a history
-- holding one is read again when a worker restarts. Every validation
-- published before this column existed reads as holding none; the scan
-- version that introduced it reads every thread again once.
ALTER TABLE claude_launch_groups ADD COLUMN unfinished_starts INTEGER NOT NULL DEFAULT 0
    CHECK(typeof(unfinished_starts) = 'integer' AND unfinished_starts >= 0);

-- Whether a launch's child check ended with a decision: 1 once the child was
-- linked or its own sources were read and found not to be the launch's, 0
-- when it ended without one (the check did not fit its allowance, or the
-- child's sources could not be read as one). Unknown for every row stored
-- before: a restarted worker looks at those once more. A launch naming a
-- session whose check has not ended with a decision keeps that session
-- checking.
ALTER TABLE claude_launch_candidates ADD COLUMN child_check_complete INTEGER
    CHECK(child_check_complete IS NULL OR child_check_complete IN (0, 1));

-- A validation compares the decoded launch command, its time and any
-- one-use binding witness. Old rows have no fingerprint and cannot establish
-- that a newly read launch is unchanged. Only this metadata digest is saved.
ALTER TABLE claude_launch_candidates ADD COLUMN launch_check_fingerprint TEXT
    CHECK(launch_check_fingerprint IS NULL OR (
        length(launch_check_fingerprint) = 64
        AND launch_check_fingerprint NOT GLOB '*[^0-9a-f]*'));
ALTER TABLE claude_launch_staged_candidates ADD COLUMN launch_check_fingerprint TEXT
    CHECK(launch_check_fingerprint IS NULL OR (
        length(launch_check_fingerprint) = 64
        AND launch_check_fingerprint NOT GLOB '*[^0-9a-f]*'));
