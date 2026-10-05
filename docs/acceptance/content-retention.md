# Content retention and explicit deletion acceptance

Unconfigured databases default to metadata-only storage for metrics and indexing.
Full-content storage requires an explicit persisted opt-in.
The persisted setting affects future canonical writes. Deleting existing content
requires a separate call after the application's confirmation flow. These tests
use synthetic disposable databases and never alter an installed app or host data.

Run `cargo test -p xt-ingest --test retention --locked --offline` and the full
storage/parser/writer/fixture suites.

| Case                  | Setup and action                                                                                                                                                     | Expected result                                                                                                                                                                                                                                           |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Default policy        | Open fresh file and memory databases without a setting; import rich records, enrich and retry through composed and legacy writes, then reopen.                       | No title, message content or tool input is acquired even when callers request content. IDs, lengths, tool counts and usage remain; usage enriches without adding a record. No implicit full-content setting is written.                                   |
| Existing database     | Seed content with explicit full-content mode, remove the setting to represent an older unconfigured database, reopen and import more data.                           | Old content remains unchanged; new content is dropped on both connections. Explicit full-content opt-in persists across reopening.                                                                                                                        |
| Persisted policy      | Explicitly opt into full-content mode and store content, open a second connection, switch to metadata-only, import/enrich and use both legacy write methods. Reopen. | Earlier content remains; every entry point refuses to acquire new content, even on the older connection. Usage still enriches. The setting survives reopen.                                                                                               |
| Explicit restriction  | Set full-content mode and submit a batch with `keep_content=false`.                                                                                                  | The explicit restriction still drops new content. Full-content mode cannot override a stricter caller.                                                                                                                                                    |
| Content inventory     | Save transcript/title/tool content and immutable plugin receipt, plus registered synthetic fire/judge fields. Switch mode, then separately purge.                    | Every registered content field is NULL. Independently snapshot every other column in every table: identities, usage, counts, source observations, receipt rows and all other structural facts stay equal. Original disposable host-file bytes stay equal. |
| Retry after purge     | Reopen in metadata-only mode, replay the original exact plugin receipt and purge again.                                                                              | Retry acknowledges the durable UUID without adding records or restoring content; measurement revision and immutable coverage remain equal. Repeated purge changes zero rows and requests no invalidation.                                                 |
| Hook failure          | A late registered owner rejects its update after earlier tables were cleared.                                                                                        | All owners roll back together; no successful outcome or invalidation escapes. The setting survives reopening. Removing the failure allows one complete purge.                                                                                             |
| Commit failure        | A registered owner's update injects a deferred foreign-key violation.                                                                                                | Final commit fails and every content/structural value remains unchanged. Retry succeeds after removing the injected trigger.                                                                                                                              |
| Invalid configuration | Store wrong JSON types or an unknown mode; register invalid, duplicate, missing, primary-key or non-nullable columns.                                                | Writes or purge fail before returning success, with bounded errors and existing content unchanged. A valid mode update can repair the setting.                                                                                                            |

The schema currently contains four transcript-content columns: `sessions.title`,
`records.content_json`, `tool_uses.input_json` and `record_previews.text`. Tool
result blocks live inside `content_json`. `record_previews.text` is the one kept
in either mode: a one-line preview of at most 280 characters of a person's whole
message or of a proven Claude Code task notification's summary (migration 17,
`cargo test -p xt-store --test record_previews`). Purge clears it with the
others, and a replay does not refill it for the same record. Structural source labels, repository identities, public PR metadata
and measurement digests remain. New fire, judge or evidence owners must register
their content fields and enforce the shared mode when their writers are added.
The synthetic owner tables here do not claim that those future features exist.

A separate transaction-race test holds the settings write lock while a second
connection attempts a canonical write. Its SQL trace must acquire `BEGIN IMMEDIATE`
before reading policy; after the lock is released it sees metadata-only mode and
stores no title. This covers a mode change concurrent with ingestion.

Every canonical write transaction adds one bounded retention-setting SELECT.
The 10,000-record regression permits 63 queries for the legacy method and 64 for
the composed batch, preserving the same requested-row prefetch and SQLite 999-bind
limit. No migration or dependency is added.

This API clears logical content fields. It does not promise forensic erasure of
SQLite free pages, WAL files, external backups or original transcripts. Changing
retention mode by itself never deletes saved content.

Desktop Settings exposes this API without changing it: a mode switch, and a
separate "Delete stored content" button behind a confirmation dialog. The app
publishes `store://content-purged` only after a committed purge that cleared
content. `cargo test -p xtrace-desktop --test privacy` and
`apps/desktop/ui/src/app/SettingsPage.test.tsx` cover this with synthetic data.
