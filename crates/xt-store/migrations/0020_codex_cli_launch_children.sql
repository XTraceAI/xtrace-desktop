-- A fourth closed kind of sub-session evidence, `codex_cli_launch`: a Codex
-- agent's own recorded `exec` operation ran one literal fresh
-- `codex exec --json` command, that operation's own first process result
-- printed exactly one `thread.started` event naming the new thread, and that
-- thread is a fresh saved Codex session whose own opening header says it was
-- started by `exec` after the launch, with nothing inherited. It is the
-- `codex_claude_launch` chain with a Codex child: the same structural
-- identifiers in the same columns, the witness `codex_exec_json_thread_started`
-- in place of `codex_exec_session_id_launch`. As for every kind, the Human
-- input view reads an accepted relation of it the same way; nothing else
-- about the child changes.
--
-- The kinds `codex_claude_launch` and `claude_bash_launch` require a Claude
-- child, so the two tables are rebuilt with their rows, indexes and triggers
-- unchanged and only their closed lists and this kind's own check widened: a
-- CHECK cannot be widened in place. The launch rows gain the host of the
-- child they name, `claude` for every row already stored. No path, command,
-- prompt, output, time or digest is kept. The managed views read the
-- relation table and are dropped first; opening the store recreates them
-- from their unchanged definitions.
DROP VIEW IF EXISTS v_response_usage;
DROP VIEW IF EXISTS v_usage_records;
DROP VIEW IF EXISTS v_session_events;
DROP VIEW IF EXISTS v_records;
DROP VIEW IF EXISTS v_human_inputs;

CREATE TABLE session_creation_relations_0020 (
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
        evidence_kind IN ('codex_thread_spawn', 'cli_artifact_create', 'codex_claude_launch',
                          'codex_cli_launch')
    ),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    witness TEXT NOT NULL CHECK(
        witness IN ('rollout_opening_session_meta', 'claude_cli_redirected_json_result',
                    'codex_exec_session_id_launch', 'codex_exec_json_thread_started')
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
    -- `codex_claude_launch` and `codex_cli_launch` only.
    completion_operation_index INTEGER,
    segment_rollout_id TEXT,
    launch_ordinal INTEGER,
    completion_ordinal INTEGER,
    UNIQUE(child_host, child_native_session_id),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(NOT (parent_host = child_host AND parent_native_session_id = child_native_session_id)),
    CHECK(evidence_kind IN ('codex_claude_launch', 'codex_cli_launch') OR (
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
    )),
    -- A Codex agent's recorded fresh `codex exec --json` launch, and its
    -- whole chain: the same fields as `codex_claude_launch`, a Codex child.
    CHECK(evidence_kind <> 'codex_cli_launch' OR (
        child_host = 'codex' AND parent_host = 'codex'
        AND witness = 'codex_exec_json_thread_started'
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
INSERT INTO session_creation_relations_0020(rowid,child_session_id,child_host,
    child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
    evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
    launch_call_id,launch_operation_index,process_session_id,completion_call_id,
    output_read_call_id,provider_result_uuid,completion_operation_index,segment_rollout_id,
    launch_ordinal,completion_ordinal)
SELECT rowid,child_session_id,child_host,child_native_session_id,parent_host,
    parent_native_session_id,evidence_kind,evidence_version,witness,state,recorded_at,
    parent_session_id,first_record_uuid,launch_call_id,launch_operation_index,
    process_session_id,completion_call_id,output_read_call_id,provider_result_uuid,
    completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal
FROM session_creation_relations;
DROP TRIGGER session_creation_relations_immutable_update;
DROP TRIGGER session_creation_relations_immutable_delete;
DROP TABLE session_creation_relations;
ALTER TABLE session_creation_relations_0020 RENAME TO session_creation_relations;
CREATE INDEX session_creation_relations_parent
    ON session_creation_relations(parent_host, parent_native_session_id);
CREATE INDEX session_creation_relations_launch
    ON session_creation_relations(parent_session_id, launch_call_id, launch_operation_index)
    WHERE evidence_kind IN ('cli_artifact_create', 'codex_claude_launch', 'codex_cli_launch');
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

-- `codex_cli_launch` child facts: a Codex thread's own recorded `exec`
-- operation ran a fresh `codex exec --json` command whose own first process
-- result named the child, a fresh saved Codex session started by `exec`. Its
-- source is that thread, the history file (rollout) the launch lies in, the
-- launch call and operation, and the child's first input.
CREATE TABLE session_child_facts_0020 (
    child_session_id TEXT NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(child_native_session_id) BETWEEN 1 AND 256
        AND child_native_session_id NOT GLOB '*[^!-~]*'
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN (
        'codex_thread_spawn', 'codex_guardian', 'codex_claude_launch', 'claude_bash_launch',
        'codex_cli_launch')),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    source_native_session_id TEXT NOT NULL CHECK(
        length(source_native_session_id) BETWEEN 1 AND 256
        AND source_native_session_id NOT GLOB '*[^!-~]*'
    ),
    source_rollout_id TEXT,
    launch_call_id TEXT,
    launch_operation_index INTEGER,
    first_record_uuid TEXT,
    state TEXT NOT NULL CHECK(state IN ('accepted', 'withheld')),
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer'),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(evidence_kind NOT IN ('codex_thread_spawn', 'codex_guardian') OR (
        child_host = 'codex' AND source_native_session_id = child_native_session_id
        AND source_rollout_id IS NULL AND launch_call_id IS NULL
        AND launch_operation_index IS NULL AND first_record_uuid IS NULL
    )),
    CHECK(evidence_kind NOT IN ('codex_claude_launch', 'claude_bash_launch') OR (
        child_host = 'claude' AND source_native_session_id <> child_native_session_id
        AND launch_call_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
    )),
    CHECK(evidence_kind <> 'codex_claude_launch' OR (
        length(source_rollout_id) = 36 AND source_rollout_id NOT GLOB '*[^0-9a-f-]*'
    )),
    CHECK(evidence_kind <> 'claude_bash_launch' OR source_rollout_id IS NULL),
    -- A Codex launch of a Codex child: another thread's call, operation and
    -- history file, and the child's first input, each a bounded token.
    CHECK(evidence_kind <> 'codex_cli_launch' OR (
        child_host = 'codex' AND source_native_session_id <> child_native_session_id
        AND launch_call_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        -- A CHECK passes on NULL, so the history file says so.
        AND source_rollout_id IS NOT NULL
        AND length(source_rollout_id) = 36 AND source_rollout_id NOT GLOB '*[^0-9a-f-]*'
    ))
);
INSERT INTO session_child_facts_0020(rowid,child_session_id,child_host,child_native_session_id,
    evidence_kind,evidence_version,source_native_session_id,source_rollout_id,launch_call_id,
    launch_operation_index,first_record_uuid,state,recorded_at)
SELECT rowid,child_session_id,child_host,child_native_session_id,evidence_kind,
    evidence_version,source_native_session_id,source_rollout_id,launch_call_id,launch_operation_index,
    first_record_uuid,state,recorded_at
FROM session_child_facts;
DROP TRIGGER session_child_facts_immutable_update;
DROP TRIGGER session_child_facts_immutable_delete;
DROP TABLE session_child_facts;
ALTER TABLE session_child_facts_0020 RENAME TO session_child_facts;
CREATE UNIQUE INDEX session_child_facts_anchor ON session_child_facts(
    child_session_id, evidence_kind, source_native_session_id,
    ifnull(source_rollout_id, ''), ifnull(launch_call_id, ''),
    ifnull(launch_operation_index, -1));
CREATE INDEX session_child_facts_launch ON session_child_facts(
    evidence_kind, source_native_session_id, launch_call_id, launch_operation_index)
    WHERE launch_call_id IS NOT NULL;
CREATE TRIGGER session_child_facts_immutable_update
BEFORE UPDATE ON session_child_facts
WHEN NOT (
    OLD.state = 'accepted' AND NEW.state = 'withheld'
    AND NEW.child_session_id IS OLD.child_session_id
    AND NEW.child_host IS OLD.child_host
    AND NEW.child_native_session_id IS OLD.child_native_session_id
    AND NEW.evidence_kind IS OLD.evidence_kind
    AND NEW.evidence_version IS OLD.evidence_version
    AND NEW.source_native_session_id IS OLD.source_native_session_id
    AND NEW.source_rollout_id IS OLD.source_rollout_id
    AND NEW.launch_call_id IS OLD.launch_call_id
    AND NEW.launch_operation_index IS OLD.launch_operation_index
    AND NEW.first_record_uuid IS OLD.first_record_uuid
    AND NEW.recorded_at IS OLD.recorded_at
)
BEGIN SELECT RAISE(ABORT, 'a child fact is immutable'); END;
CREATE TRIGGER session_child_facts_immutable_delete
BEFORE DELETE ON session_child_facts
BEGIN SELECT RAISE(ABORT, 'a child fact is immutable'); END;

-- Which host the child a found launch names is on: `claude` for a
-- `--session-id` launch, every row stored before this migration; `codex` for
-- a `codex exec --json` launch, whose child the launch's own result named.
ALTER TABLE claude_launch_candidates
    ADD COLUMN child_host TEXT NOT NULL DEFAULT 'claude' CHECK(child_host IN ('claude', 'codex'));
ALTER TABLE claude_launch_staged_candidates
    ADD COLUMN child_host TEXT NOT NULL DEFAULT 'claude' CHECK(child_host IN ('claude', 'codex'));
