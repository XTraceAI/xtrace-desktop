-- Human-only assumptions and length adjustments. Canonical records are unchanged.
CREATE TABLE human_session_origins (
 session_id TEXT PRIMARY KEY REFERENCES sessions(session_id),
 host TEXT NOT NULL, native_session_id TEXT NOT NULL,
 parent_host TEXT NOT NULL, parent_native_session_id TEXT NOT NULL,
 method TEXT NOT NULL CHECK(method IN ('returned_child_id','explicit_session_id','artifact_result_chain','copied_worker_result','exact_initial_text','native_reviewer_header')),
 evidence_id TEXT NOT NULL, launch_id TEXT NOT NULL,
 rule_version INTEGER NOT NULL CHECK(rule_version=1),
 conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0,1)),
 conflict_reason TEXT CHECK(conflict_reason IN ('saved_binding_mismatch','evidence_mismatch')),
 CHECK(host<>parent_host OR native_session_id<>parent_native_session_id)
);
CREATE TABLE human_input_adjustments (
 record_uuid TEXT PRIMARY KEY, session_id TEXT NOT NULL,
 original_ts TEXT NOT NULL, original_length INTEGER NOT NULL CHECK(original_length>=0),
 retained_length INTEGER CHECK(retained_length>=0 AND retained_length<=original_length),
 reason TEXT NOT NULL CHECK(reason IN ('heartbeat','selected_skill','question_reply','image_wrapper')),
 rule_version INTEGER NOT NULL CHECK(rule_version=1),
 native_item_id TEXT,
 conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0,1)),
 conflict_reason TEXT CHECK(conflict_reason IN ('saved_binding_mismatch','evidence_mismatch')),
 FOREIGN KEY(record_uuid,session_id) REFERENCES records(uuid,session_id)
);
