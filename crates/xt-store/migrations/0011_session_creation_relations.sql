-- A structural fact that one indexed user session was created by another
-- session's agent: a sub-session. It is a navigation projection only. No
-- metric, span, usage, search, membership or ordering query reads it; the
-- child keeps its own row, identity, detail route and every measurement.
--
-- One row per exact canonical child. It keeps identities and closed labels
-- only: the child's host and native identity, checked against the indexed
-- session when the row is written; the parent's host and full native identity
-- as the evidence named it; the evidence kind and its version; and which
-- structural witness carried it. No prompt, command, output, task path, role,
-- title or digest of any of them is stored.
--
-- The parent need not be indexed. A read resolves it to exactly one indexed
-- user session and shows it only then. A later proof naming a different
-- parent for the same child never replaces the first: the row becomes
-- `conflicted` and every read withholds it. Nothing else may change, and no
-- row may be deleted.
--
-- Two closed kinds of evidence exist. `codex_thread_spawn`: a Codex thread's
-- own rollout opens by naming the thread that spawned it. `cli_artifact_create`:
-- a trusted validator proved that one Codex agent's recorded foreground
-- `claude -p` launch created a Claude session, from the launch, its own
-- process and successful completion, the JSON result that process wrote
-- naming the child, the parent's later read of that result, and the child's
-- first input being exactly what the launch submitted. Only the structural
-- identifiers of that chain are kept, in columns that exist for this kind
-- alone: the exact canonical parent, the child's first input record, the
-- launch call and its operation, the process handle, the completion and read
-- calls and the provider's result identifier. No path, command, prompt,
-- result body, digest or timestamp of the chain is kept. One launch operation
-- names at most one child: a second child claimed for it withholds every
-- claim on it, which `cli_artifact_launch_owners` below remembers.
CREATE TABLE session_creation_relations (
    child_session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex', 'cursor', 'other')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(trim(child_native_session_id)) > 0 AND length(child_native_session_id) <= 256
    ),
    parent_host TEXT NOT NULL CHECK(parent_host IN ('claude', 'codex', 'cursor', 'other')),
    parent_native_session_id TEXT NOT NULL CHECK(
        length(trim(parent_native_session_id)) > 0 AND length(parent_native_session_id) <= 256
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('codex_thread_spawn', 'cli_artifact_create')),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    witness TEXT NOT NULL CHECK(
        witness IN ('rollout_opening_session_meta', 'claude_cli_redirected_json_result')
    ),
    state TEXT NOT NULL CHECK(state IN ('accepted', 'conflicted')),
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer'),
    -- `cli_artifact_create` only; each is a bounded printable token with no
    -- space, never text.
    parent_session_id TEXT,
    first_record_uuid TEXT,
    launch_call_id TEXT,
    launch_operation_index INTEGER,
    process_session_id TEXT,
    completion_call_id TEXT,
    output_read_call_id TEXT,
    provider_result_uuid TEXT,
    UNIQUE(child_host, child_native_session_id),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(NOT (parent_host = child_host AND parent_native_session_id = child_native_session_id)),
    -- A native Codex spawn names a Codex thread from that thread's own
    -- rollout header, and nothing else.
    CHECK(evidence_kind <> 'codex_thread_spawn' OR (
        child_host = 'codex' AND parent_host = 'codex'
        AND witness = 'rollout_opening_session_meta'
        AND parent_session_id IS NULL AND first_record_uuid IS NULL
        AND launch_call_id IS NULL AND launch_operation_index IS NULL
        AND process_session_id IS NULL AND completion_call_id IS NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
    )),
    -- A foreground Claude CLI launch by a Codex agent, and its whole witness.
    CHECK(evidence_kind <> 'cli_artifact_create' OR (
        child_host = 'claude' AND parent_host = 'codex'
        AND witness = 'claude_cli_redirected_json_result'
        -- A CHECK passes on NULL, so every required field says so.
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
    ))
);
CREATE INDEX session_creation_relations_parent
    ON session_creation_relations(parent_host, parent_native_session_id);
-- Finding every child claimed by one parent launch operation. Not unique: a
-- second claim must be stored so that both are withheld, not refused while
-- the first stays shown.
CREATE INDEX session_creation_relations_launch
    ON session_creation_relations(parent_session_id, launch_call_id, launch_operation_index)
    WHERE evidence_kind = 'cli_artifact_create';
-- Resolving a parent's host/native identity to its indexed sessions.
CREATE INDEX sessions_host_native ON sessions(host, native_session_id);
-- Finding every root rollout recorded for one Codex thread by name. A root
-- locator ends in `<thread>.jsonl`, 42 characters, so one exact seek finds
-- the thread's root and any duplicate without scanning other locators.
CREATE INDEX source_cursors_tail ON source_cursors(source, substr(cursor_key, -42));
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
)
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;
CREATE TRIGGER session_creation_relations_immutable_delete
BEFORE DELETE ON session_creation_relations
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;

-- Which child each foreground CLI launch operation was first claimed for. A
-- relation row keeps only its child's first launch, so it cannot tell that a
-- launch it never kept was later claimed for another child. Every proof the
-- store validates claims its launch here first, whatever then becomes of its
-- relation; a claim for a different child marks the launch disputed, which
-- withholds every relation resting on it or on its first child, now and for
-- every later claim. The first child never changes, `disputed` only turns on,
-- and no row is deleted. Structural identifiers only.
CREATE TABLE cli_artifact_launch_owners (
    parent_session_id TEXT NOT NULL CHECK(
        length(parent_session_id) BETWEEN 1 AND 256 AND parent_session_id NOT GLOB '*[^!-~]*'
    ),
    launch_call_id TEXT NOT NULL CHECK(
        length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'
    ),
    launch_operation_index INTEGER NOT NULL CHECK(
        typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
    ),
    first_child_session_id TEXT NOT NULL REFERENCES sessions(session_id),
    disputed INTEGER NOT NULL CHECK(typeof(disputed) = 'integer' AND disputed IN (0, 1)),
    PRIMARY KEY(parent_session_id, launch_call_id, launch_operation_index)
) WITHOUT ROWID;
CREATE TRIGGER cli_artifact_launch_owners_immutable_update
BEFORE UPDATE ON cli_artifact_launch_owners
WHEN NOT (
    OLD.disputed = 0 AND NEW.disputed = 1
    AND NEW.parent_session_id IS OLD.parent_session_id
    AND NEW.launch_call_id IS OLD.launch_call_id
    AND NEW.launch_operation_index IS OLD.launch_operation_index
    AND NEW.first_child_session_id IS OLD.first_child_session_id
)
BEGIN SELECT RAISE(ABORT, 'a launch owner is immutable'); END;
CREATE TRIGGER cli_artifact_launch_owners_immutable_delete
BEFORE DELETE ON cli_artifact_launch_owners
BEGIN SELECT RAISE(ABORT, 'a launch owner is immutable'); END;

-- How far the one-time pass over already-indexed sources has read, per
-- evidence kind and version: the last source locator it finished, in key
-- order, and whether it reached the end. A new table cannot wait for new
-- events to learn about sessions indexed before it existed; this lets that
-- pass stop at any point and resume there. It holds no content.
CREATE TABLE session_creation_bootstrap (
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('codex_thread_spawn')),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    after_locator TEXT,
    complete INTEGER NOT NULL CHECK(complete IN (0, 1)),
    PRIMARY KEY(evidence_kind, evidence_version)
);
