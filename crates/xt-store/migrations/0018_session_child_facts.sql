-- That an indexed user session was created by an agent, recorded apart from
-- who that agent was: a sub-session whose parent may be unknown. It feeds the
-- known-child bit of the Sessions and Dashboard reads only. No metric, span,
-- usage, Human input, search, membership or ordering query reads it, and it
-- never names, guesses or replaces a parent: parents live only in
-- `session_creation_relations` and `human_session_origins`, unchanged.
--
-- One row per piece of evidence. It keeps identities and closed labels only:
-- the exact canonical child with its host and native identity, checked
-- against the indexed user session when the row is written; the closed kind
-- of evidence and its version; and the structural identifiers of where the
-- evidence lies. No prompt, command, answer, path, title or digest is stored.
--
-- `codex_thread_spawn` and `codex_guardian`: the Codex thread's own opening
-- `session_meta` names it, by its typed `source`, a spawned or Guardian
-- subagent. The evidence lies in the child's own header, so its source is the
-- child itself and nothing else is kept.
-- `codex_claude_launch`: a Codex thread's own recorded `exec` operation ran a
-- create-mode Claude command naming the child with `--session-id`, that
-- operation's own process acknowledged it, and the child is a fresh saved
-- session. Its source is that thread, the history file (rollout) the launch
-- lies in, the launch call and operation, and the child's first input.
-- `claude_bash_launch`: a Claude session's own `Bash` call launched a Claude
-- print session in a literal directory with a literal prompt, its own
-- closing result printed that launch's complete answer, and the child is the
-- one fresh session in that directory born within the call whose first input
-- and whole first answer are those. Its source is that Claude session, the
-- call and the launch's position in it, and the child's first input.
--
-- Independent evidence for one child adds rows; none contradicts another. One
-- launch names at most one child: a second child claimed by the same launch,
-- or a second first input for the same launch and child, withholds every row
-- of that launch, now and for every later claim. A withheld row never shows
-- again; another accepted row of the child still does. Only that change is
-- allowed, and no row is deleted.
CREATE TABLE session_child_facts (
    child_session_id TEXT NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(child_native_session_id) BETWEEN 1 AND 256
        AND child_native_session_id NOT GLOB '*[^!-~]*'
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN (
        'codex_thread_spawn', 'codex_guardian', 'codex_claude_launch', 'claude_bash_launch')),
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
    -- A typed header is the child's own: no launch, no other source.
    CHECK(evidence_kind NOT IN ('codex_thread_spawn', 'codex_guardian') OR (
        child_host = 'codex' AND source_native_session_id = child_native_session_id
        AND source_rollout_id IS NULL AND launch_call_id IS NULL
        AND launch_operation_index IS NULL AND first_record_uuid IS NULL
    )),
    -- A launch is another session's: its call, operation and the child's
    -- first input, each a bounded printable token.
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
    CHECK(evidence_kind <> 'claude_bash_launch' OR source_rollout_id IS NULL)
);
-- One row per child and evidence anchor: a replay finds it.
CREATE UNIQUE INDEX session_child_facts_anchor ON session_child_facts(
    child_session_id, evidence_kind, source_native_session_id,
    ifnull(source_rollout_id, ''), ifnull(launch_call_id, ''),
    ifnull(launch_operation_index, -1));
-- Every child one launch was claimed for.
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
