# Indexed Sessions browser

The Sessions route reads the local index through a generated Rust DTO. It shows
host, a shortened session ID, repository/branch, observed model, first recorded
time, indexed record count and identity-conflict status. It does not read source
transcripts or select stored content or titles.

Search matches literal substrings in IDs, repository paths and branches. Host
filters and descending time/ID pagination run in SQLite, with 50 rows per page.
Unknown metadata remains unknown. Record counts are indexed user/assistant
records, including tool-result carriers; they are not human-message counts.
The current view does not include product time/token metrics, PR filtering or
session-detail navigation. A PR-filter URL explicitly explains that limitation.

## Verification and expected results

- `cargo test -p xt-store --test session_list --locked`: page through more than
  two pages with tied/unknown dates without duplicates or omissions; verify host
  and literal-substring filters, unknown metadata and rejected invalid filters.
- `cargo test -p xtrace-desktop --test fixture_mode --all-features --locked`:
  the native fixture export and committed browser fixture agree on session rows.
- `pnpm check`: typed DataSource contract, metadata rendering, host/search resets,
  load-more, import-event refresh and query-error recovery pass.
- `pnpm e2e --grep 'indexed Sessions'`: fixture metadata and filters render in
  Chromium and WebKit, in light and dark themes.
- Local native validation: `pnpm check:native --base <reviewed-base-sha>`.
  In a normal native launch, Sessions must display the existing local index,
  show incomplete coverage when reported by the indexer, and load subsequent
  pages. Keep real-history evidence local; use synthetic fixtures for PR images.

No new hosted CI job or browser installation step is added by this change.
