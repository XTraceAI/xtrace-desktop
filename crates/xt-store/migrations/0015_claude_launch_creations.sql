-- A third closed kind of sub-session evidence, `codex_claude_launch`: a Codex
-- agent's own recorded `exec` operation ran one literal create-mode Claude
-- print command that named its new session with `--session-id`, that
-- operation's own process exited 0, and the named Claude session's first
-- input is exactly what the command submitted, dated between the launch and
-- the exit. Like the other kinds it is a creation relation: the Human input
-- view reads every accepted relation the same way, so the user messages of a
-- child it relates are, as for any agent-created session, not counted as
-- typed by a person. Nothing else about the child changes.
--
-- The relation table is rebuilt with its rows unchanged, because a CHECK
-- cannot be widened in place. The new kind keeps only structural identifiers,
-- in columns that exist for it alone or that it shares with
-- `cli_artifact_create`: the exact canonical parent, the child's first input
-- record, the launch call and its operation, the process handle when the
-- process outlived its launch, the completion call and its operation, the
-- physical history file (rollout) the chain lies in, and the rows' ordinals
-- in a paginated history. No path, command, prompt, output, time or digest
-- is kept. The managed views read this table and are dropped first; opening
-- the store recreates them from their unchanged definitions.
DROP VIEW IF EXISTS v_response_usage;
DROP VIEW IF EXISTS v_usage_records;
DROP VIEW IF EXISTS v_session_events;
DROP VIEW IF EXISTS v_records;
DROP VIEW IF EXISTS v_human_inputs;

CREATE TABLE session_creation_relations_0015 (
    child_session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex', 'cursor', 'other')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(trim(child_native_session_id)) > 0 AND length(child_native_session_id) <= 256
    ),
    parent_host TEXT NOT NULL CHECK(parent_host IN ('claude', 'codex', 'cursor', 'other')),
    parent_native_session_id TEXT NOT NULL CHECK(
        length(trim(parent_native_session_id)) > 0 AND length(parent_native_session_id) <= 256
    ),
    evidence_kind TEXT NOT NULL CHECK(
        evidence_kind IN ('codex_thread_spawn', 'cli_artifact_create', 'codex_claude_launch')
    ),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    witness TEXT NOT NULL CHECK(
        witness IN ('rollout_opening_session_meta', 'claude_cli_redirected_json_result',
                    'codex_exec_session_id_launch')
    ),
    state TEXT NOT NULL CHECK(state IN ('accepted', 'conflicted')),
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer'),
    parent_session_id TEXT,
    first_record_uuid TEXT,
    launch_call_id TEXT,
    launch_operation_index INTEGER,
    process_session_id TEXT,
    completion_call_id TEXT,
    output_read_call_id TEXT,
    provider_result_uuid TEXT,
    -- `codex_claude_launch` only.
    completion_operation_index INTEGER,
    segment_rollout_id TEXT,
    launch_ordinal INTEGER,
    completion_ordinal INTEGER,
    UNIQUE(child_host, child_native_session_id),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(NOT (parent_host = child_host AND parent_native_session_id = child_native_session_id)),
    CHECK(evidence_kind = 'codex_claude_launch' OR (
        completion_operation_index IS NULL AND segment_rollout_id IS NULL
        AND launch_ordinal IS NULL AND completion_ordinal IS NULL
    )),
    CHECK(evidence_kind <> 'codex_thread_spawn' OR (
        child_host = 'codex' AND parent_host = 'codex'
        AND witness = 'rollout_opening_session_meta'
        AND parent_session_id IS NULL AND first_record_uuid IS NULL
        AND launch_call_id IS NULL AND launch_operation_index IS NULL
        AND process_session_id IS NULL AND completion_call_id IS NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
    )),
    CHECK(evidence_kind <> 'cli_artifact_create' OR (
        child_host = 'claude' AND parent_host = 'codex'
        AND witness = 'claude_cli_redirected_json_result'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL AND process_session_id IS NOT NULL
        AND completion_call_id IS NOT NULL AND output_read_call_id IS NOT NULL
        AND provider_result_uuid IS NOT NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(process_session_id) BETWEEN 1 AND 256
        AND process_session_id NOT GLOB '*[^!-~]*'
        AND length(completion_call_id) BETWEEN 1 AND 256
        AND completion_call_id NOT GLOB '*[^!-~]*'
        AND length(output_read_call_id) BETWEEN 1 AND 256
        AND output_read_call_id NOT GLOB '*[^!-~]*'
        AND length(provider_result_uuid) BETWEEN 1 AND 256
        AND provider_result_uuid NOT GLOB '*[^!-~]*'
    )),
    -- A Codex agent's recorded `--session-id` launch, and its whole chain.
    -- The process handle is absent when the process exited within its
    -- launch; no result is ever read, so no read call or result identifier.
    CHECK(evidence_kind <> 'codex_claude_launch' OR (
        child_host = 'claude' AND parent_host = 'codex'
        AND witness = 'codex_exec_session_id_launch'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL AND completion_call_id IS NOT NULL
        AND segment_rollout_id IS NOT NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND typeof(completion_operation_index) = 'integer' AND completion_operation_index >= 0
        AND ((launch_ordinal IS NULL AND completion_ordinal IS NULL) OR (
            typeof(launch_ordinal) = 'integer' AND typeof(completion_ordinal) = 'integer'
            AND launch_ordinal >= 0 AND completion_ordinal > launch_ordinal))
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
        AND (process_session_id IS NULL OR (length(process_session_id) BETWEEN 1 AND 256
            AND process_session_id NOT GLOB '*[^!-~]*'))
        AND length(completion_call_id) BETWEEN 1 AND 256
        AND completion_call_id NOT GLOB '*[^!-~]*'
        AND length(segment_rollout_id) = 36
        AND segment_rollout_id NOT GLOB '*[^0-9a-f-]*'
    ))
);
INSERT INTO session_creation_relations_0015(child_session_id,child_host,
    child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
    evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
    launch_call_id,launch_operation_index,process_session_id,completion_call_id,
    output_read_call_id,provider_result_uuid)
SELECT child_session_id,child_host,child_native_session_id,parent_host,
    parent_native_session_id,evidence_kind,evidence_version,witness,state,recorded_at,
    parent_session_id,first_record_uuid,launch_call_id,launch_operation_index,
    process_session_id,completion_call_id,output_read_call_id,provider_result_uuid
FROM session_creation_relations;
DROP TRIGGER session_creation_relations_immutable_update;
DROP TRIGGER session_creation_relations_immutable_delete;
DROP TABLE session_creation_relations;
ALTER TABLE session_creation_relations_0015 RENAME TO session_creation_relations;
CREATE INDEX session_creation_relations_parent
    ON session_creation_relations(parent_host, parent_native_session_id);
CREATE INDEX session_creation_relations_launch
    ON session_creation_relations(parent_session_id, launch_call_id, launch_operation_index)
    WHERE evidence_kind IN ('cli_artifact_create', 'codex_claude_launch');
CREATE TRIGGER session_creation_relations_immutable_update
BEFORE UPDATE ON session_creation_relations
WHEN NOT (
    OLD.state = 'accepted' AND NEW.state = 'conflicted'
    AND NEW.child_session_id IS OLD.child_session_id
    AND NEW.child_host IS OLD.child_host
    AND NEW.child_native_session_id IS OLD.child_native_session_id
    AND NEW.parent_host IS OLD.parent_host
    AND NEW.parent_native_session_id IS OLD.parent_native_session_id
    AND NEW.evidence_kind IS OLD.evidence_kind
    AND NEW.evidence_version IS OLD.evidence_version
    AND NEW.witness IS OLD.witness
    AND NEW.recorded_at IS OLD.recorded_at
    AND NEW.parent_session_id IS OLD.parent_session_id
    AND NEW.first_record_uuid IS OLD.first_record_uuid
    AND NEW.launch_call_id IS OLD.launch_call_id
    AND NEW.launch_operation_index IS OLD.launch_operation_index
    AND NEW.process_session_id IS OLD.process_session_id
    AND NEW.completion_call_id IS OLD.completion_call_id
    AND NEW.output_read_call_id IS OLD.output_read_call_id
    AND NEW.provider_result_uuid IS OLD.provider_result_uuid
    AND NEW.completion_operation_index IS OLD.completion_operation_index
    AND NEW.segment_rollout_id IS OLD.segment_rollout_id
    AND NEW.launch_ordinal IS OLD.launch_ordinal
    AND NEW.completion_ordinal IS OLD.completion_ordinal
)
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;
CREATE TRIGGER session_creation_relations_immutable_delete
BEFORE DELETE ON session_creation_relations
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;

-- One validation of one Codex thread's whole history for launches. A
-- validation reads every physical file of the thread's history (the
-- original rollout the index records and every continuation a name-only
-- census found) and checks them together: structure, ordinals, every launch
-- chain and, across all files, that each call identifier of a chain occurs
-- exactly once. Its result is published at once, with the exact member files
-- and their generations, as one revision.
--
-- `revision`: the published validation, or NULL before the first.
-- `allocated`: the last revision number any validation attempt was given; an
-- attempt publishes only its own number, so an abandoned attempt can never
-- publish later. `status`: `pending` while a replacement validation runs (it
-- is set before any slower work, and no launch of the thread is linked while
-- pending), `valid`, or `invalid` (a member broke the history's structure,
-- or the history is beyond this version's bounds: nothing of it is linked).
-- Numbers and closed labels only.
CREATE TABLE claude_launch_groups (
    parent_native_session_id TEXT PRIMARY KEY NOT NULL CHECK(
        length(parent_native_session_id) = 36 AND parent_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    revision INTEGER CHECK(revision IS NULL OR (
        typeof(revision) = 'integer' AND revision > 0 AND revision <= allocated)),
    allocated INTEGER NOT NULL CHECK(typeof(allocated) = 'integer' AND allocated >= 0),
    status TEXT NOT NULL CHECK(status IN ('pending', 'valid', 'invalid')),
    validator_version INTEGER NOT NULL CHECK(
        typeof(validator_version) = 'integer' AND validator_version > 0),
    CHECK(revision IS NOT NULL OR status = 'pending')
) WITHOUT ROWID;

-- The exact member files of a thread's published validation: each file's
-- rollout identity and generation (device, inode, length, modification and
-- change times), how far its complete lines went, and whether it was a valid
-- member (a valid member may hold no launch) or broke the structure. The set
-- is replaced as a whole when a validation is published. A member whose file
-- is no longer exactly this generation, or a file the census finds that is
-- not a member, makes the thread validated again.
CREATE TABLE claude_launch_group_members (
    parent_native_session_id TEXT NOT NULL CHECK(length(parent_native_session_id) = 36 AND parent_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    rollout_id TEXT NOT NULL CHECK(length(rollout_id) = 36 AND rollout_id NOT GLOB '*[^0-9a-f-]*'),
    revision INTEGER NOT NULL CHECK(typeof(revision) = 'integer' AND revision > 0),
    segment_device INTEGER NOT NULL CHECK(typeof(segment_device) = 'integer'),
    segment_inode INTEGER NOT NULL CHECK(typeof(segment_inode) = 'integer'),
    segment_length INTEGER NOT NULL CHECK(typeof(segment_length) = 'integer' AND segment_length >= 0),
    segment_mtime_ns INTEGER NOT NULL CHECK(typeof(segment_mtime_ns) = 'integer'),
    segment_ctime_ns INTEGER NOT NULL CHECK(typeof(segment_ctime_ns) = 'integer'),
    scanned_length INTEGER NOT NULL CHECK(
        typeof(scanned_length) = 'integer' AND scanned_length >= 0 AND scanned_length <= segment_length),
    status TEXT NOT NULL CHECK(status IN ('valid', 'invalid')),
    PRIMARY KEY(parent_native_session_id, rollout_id)
) WITHOUT ROWID;

-- The launches a published validation found, one row per launch operation,
-- waiting for (or matched with) the Claude session each named. The row's
-- structural fields locate its two recorded lines; `source_revision` is the
-- validation that found it and `source_verdict` its parent-side verdict
-- (`rejected` when its recorded lines no longer read as that launch);
-- `child_state` is the child side: `waiting` (not indexed; its import or the
-- next start looks again), `retry` (a source was busy or changed; a timed
-- pass looks again), `linked` (a proof went to the store), `rejected` (the
-- child's transcript disagrees; its next import looks again). Identifiers,
-- positions and ordinals only: no path, prompt, command, output, time or
-- digest.
CREATE TABLE claude_launch_candidates (
    parent_native_session_id TEXT NOT NULL CHECK(length(parent_native_session_id) = 36 AND parent_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    rollout_id TEXT NOT NULL CHECK(length(rollout_id) = 36 AND rollout_id NOT GLOB '*[^0-9a-f-]*'),
    launch_call_id TEXT NOT NULL CHECK(length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'),
    launch_operation_index INTEGER NOT NULL CHECK(typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0),
    child_native_session_id TEXT NOT NULL CHECK(length(child_native_session_id) = 36 AND child_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    completion_call_id TEXT NOT NULL CHECK(length(completion_call_id) BETWEEN 1 AND 256 AND completion_call_id NOT GLOB '*[^!-~]*'),
    completion_operation_index INTEGER NOT NULL CHECK(
        typeof(completion_operation_index) = 'integer' AND completion_operation_index >= 0),
    process_session_id TEXT CHECK(process_session_id IS NULL OR (
        length(process_session_id) BETWEEN 1 AND 32 AND process_session_id NOT GLOB '*[^0-9]*')),
    launch_offset INTEGER NOT NULL CHECK(typeof(launch_offset) = 'integer' AND launch_offset >= 0),
    completion_offset INTEGER NOT NULL CHECK(
        typeof(completion_offset) = 'integer' AND completion_offset > launch_offset),
    launch_ordinal INTEGER CHECK(launch_ordinal IS NULL OR (
        typeof(launch_ordinal) = 'integer' AND launch_ordinal >= 0)),
    completion_ordinal INTEGER CHECK(completion_ordinal IS NULL OR (
        typeof(completion_ordinal) = 'integer' AND completion_ordinal > launch_ordinal)),
    source_revision INTEGER NOT NULL CHECK(typeof(source_revision) = 'integer' AND source_revision > 0),
    source_verdict TEXT NOT NULL CHECK(source_verdict IN ('valid', 'rejected')),
    child_state TEXT NOT NULL CHECK(child_state IN ('waiting', 'retry', 'linked', 'rejected')),
    CHECK((launch_ordinal IS NULL) = (completion_ordinal IS NULL)),
    PRIMARY KEY(parent_native_session_id, rollout_id, launch_call_id, launch_operation_index)
) WITHOUT ROWID;
CREATE INDEX claude_launch_candidates_child
    ON claude_launch_candidates(child_native_session_id, child_state);
CREATE INDEX claude_launch_candidates_state
    ON claude_launch_candidates(child_state);

-- A validation attempt's launches, written in bounded chunks under the
-- attempt's allocated revision before it is published. Nothing reads them as
-- launches: publication moves the whole set into
-- `claude_launch_candidates` at once, and a new attempt deletes what an
-- abandoned one left.
CREATE TABLE claude_launch_staged_candidates (
    parent_native_session_id TEXT NOT NULL CHECK(length(parent_native_session_id) = 36 AND parent_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    revision INTEGER NOT NULL CHECK(typeof(revision) = 'integer' AND revision > 0),
    rollout_id TEXT NOT NULL CHECK(length(rollout_id) = 36 AND rollout_id NOT GLOB '*[^0-9a-f-]*'),
    launch_call_id TEXT NOT NULL CHECK(length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'),
    launch_operation_index INTEGER NOT NULL CHECK(typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0),
    child_native_session_id TEXT NOT NULL CHECK(length(child_native_session_id) = 36 AND child_native_session_id NOT GLOB '*[^0-9a-f-]*'),
    completion_call_id TEXT NOT NULL CHECK(length(completion_call_id) BETWEEN 1 AND 256 AND completion_call_id NOT GLOB '*[^!-~]*'),
    completion_operation_index INTEGER NOT NULL CHECK(
        typeof(completion_operation_index) = 'integer' AND completion_operation_index >= 0),
    process_session_id TEXT CHECK(process_session_id IS NULL OR (
        length(process_session_id) BETWEEN 1 AND 32 AND process_session_id NOT GLOB '*[^0-9]*')),
    launch_offset INTEGER NOT NULL CHECK(typeof(launch_offset) = 'integer' AND launch_offset >= 0),
    completion_offset INTEGER NOT NULL CHECK(
        typeof(completion_offset) = 'integer' AND completion_offset > launch_offset),
    launch_ordinal INTEGER CHECK(launch_ordinal IS NULL OR (
        typeof(launch_ordinal) = 'integer' AND launch_ordinal >= 0)),
    completion_ordinal INTEGER CHECK(completion_ordinal IS NULL OR (
        typeof(completion_ordinal) = 'integer' AND completion_ordinal > launch_ordinal)),
    CHECK((launch_ordinal IS NULL) = (completion_ordinal IS NULL)),
    PRIMARY KEY(parent_native_session_id, revision, rollout_id, launch_call_id,
                launch_operation_index)
) WITHOUT ROWID;
