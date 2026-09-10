# Default local storage to metrics and indexing

Status: accepted, 2026-09-10.

## Decision

An absent content-retention setting means metadata-only. Canonical ingestion
computes structural measurements from incoming content in memory, then discards
transcript text, tool input/output and transcript-derived titles before storage.
SQLite keeps the identities, timestamps, counts, usage and source provenance used
for indexing, metrics, deduplication and committed import acknowledgements.
These local metadata can still contain repository and session identities; this
policy does not claim that the database contains no sensitive information.

Full-content archival remains an explicit persisted opt-in. Existing saved
content and explicit preferences remain unchanged. Changing the mode affects
future writes only; deleting existing content remains a separate confirmed action.
Original host files are never changed by either operation.

## Consequences

Normal metrics and indexing must work without saved transcript bodies. Future
transcript views should resolve the original local source on demand, validate its
session identity, and display an unavailable-source state when it cannot be read.
Viewing a source must not silently enable archival or send content to a model or
cloud service. This storage API change does not implement that viewer or Settings UI.

Tests cover the absent setting on fresh and previously populated databases,
legacy and composed writes, enrichment, retries, explicit opt-in and reopening.
The existing purge, concurrency and rollback tests remain required. Fixture
builders requesting full content must opt in explicitly; empty fixtures use the
production default.
