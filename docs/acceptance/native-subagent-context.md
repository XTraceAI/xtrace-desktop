# Native Codex context and ledger correction

The pinned Codex reader distinguishes inherited model context with `isMeta` and
identifies authoritative usage with native response IDs. Existing indexes can
contain older classifications and UI-meter counters for the same stable IDs.

Only a discovered Codex `readers_cli` batch may correct those cached facts. It
can classify a matching record as metadata, or replace legacy usage/model facts
when a complete native ledger first supplies a response ID. Ownership, the
remaining known measurements and retained content must agree. An existing
response ID does not authorize another counter replacement. Generic imports and
plugin receipts retain the ordinary conflict rules.

Corrections and their invalidation events share the existing transaction. An
invalid later record rolls back earlier corrections. Raw rows remain stored;
Sessions counts, model labels and time ranges exclude metadata. Listing ranges
are derived in one read snapshot even for untouched older sessions, without
rewriting their cached timestamps; only the bounded page needs precise first-time
selection. No source file,
transcript-retention policy or database schema is changed.

Validation covers inherited-context classification, first-ledger correction,
generic-input refusal, incompatible content, rollback, idempotent replay and
immutable established-ledger counters. A two-record fixture retains both rows
while exposing only its one work record in Sessions. The companion upstream
reader change is [agent-plugins #238](https://github.com/XTraceAI/agent-plugins/pull/238).
