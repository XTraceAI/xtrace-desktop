# Claude live status — bounded UI acceptance

## Native integration and final checks

Claude status now comes from its local macOS session registry, not new hooks or
recent-activity timestamps. The adapter supports the verified Claude Code
versions `2.1.280` and `2.1.284` and the `interactive` and `bg` record kinds.
Other versions, missing records, ambiguous identities and failed verification
return Unknown. The native index must identify the exact unique user session,
with matching host, saved session ID and native session ID. Saved metadata
conflicts, such as different branch names, do not block live status for Claude
or Codex and their conflict flags remain saved. Host/native ID/kind mismatches
and duplicate user identities still block live status. Native process and
source validation are unchanged. A parent's state is never copied to its child.

The adapter reads only bounded PID-named JSON metadata files. It does not open
authentication key files, read conversations or process arguments, change
Claude settings, or send commands to Claude. Kernel user/process-birth checks
and safe file checks establish the holder; the published status establishes
Running or Idle. Known waiting reasons map to the existing waiting states.
No timestamp age or process presence alone establishes Running.

The existing four-view, 15-second leases and 16-target limit are shared by
Claude and Codex. Fixture and preview sources still make no native live claim.
Arc styling, counts, parent links and the existing Codex peer observer remain
unchanged. The shared PR-icon and loading-mark changes were preserved when
merging `integration/current-app`.

Historical checks before the approved conflict-gate follow-up: 51 focused
native tests passed, including 17 Claude adapter tests;
DTO parity and two gate tests passed; all 834 UI tests passed after correcting
two stale Compactions-column assertions that also failed on the shared base.
All 16 mixed-host arc browser cases passed after the PR-icon merge. Independent
scope and code reviews accepted the production change and merge resolution.

The explicitly invoked, ignored-by-default native acceptance test observed
Running then Idle from a controlled, no-tools Claude process while that same
process stayed alive. It used the production State/adapter path and a temporary
index, not the user's index. Only status metadata was printed; no conversation
was saved. This verifies one controlled transition, not every running session.
The earlier UI-only acceptance below records its separate checks and limits.

## Approved follow-up: saved metadata conflicts

The shared Claude/Codex State read now ignores the blanket saved metadata
conflict flag when deciding which exact sessions the native observer may check.
Both eligibility tests create a `main` versus `feature` branch conflict through
`Store::upsert_session`. They accept the exact user session and verify that the
saved conflict flag stays set through accepted reads, host/native ID/kind
mismatch refusals and duplicate user identity refusals. No conflict flags are
cleared, and no Store schema, adapter, process validation or UI behavior changed.

Local native checks for this follow-up passed: 2 `live_status` eligibility tests,
57 `live_codex_status` tests and 30 DTO tests. The manual Claude transition and
real Codex socket tests remained ignored. The first status-suite run passed 44
tests but failed 13 fake-peer tests because the sandbox blocked local socket
creation; rerunning the suite with socket access passed all 57 enabled tests.
The rebuild used `CARGO_BUILD_JOBS=4` and offline dependencies. Generated DTO
parity and both DTO gate tests also passed.

Temporarily restoring the old conflict gate made both updated eligibility
regressions fail at their accepted-session assertions. The approved gate removal
was then restored and both tests passed again. These assertions check which
sessions may be observed; they do not prove Running from requested targets.
The earlier controlled Running-to-Idle proof above remains a separate result.
This follow-up did not run the manual live tests, inspect the installed app,
install a build, move the shared app branch or publish anything. Independent
scope and code reviews reported no required changes.

## Approved follow-up: Dashboard mutex blocked live status

Live Claude/Codex identity reads now use a separate narrow read-only connection
to the exact live database. They no longer take the database mutex held by
Dashboard coverage. The connection opens lazily, uses SQLite read-only flags
with no create or URI flag, and skips Store writer configuration, migrations,
view updates and journal-mode changes. Fixture state never opens it. Shutdown
closes it explicitly after closing live-status observation.

The existing indexed `user_sessions_with_native` query is shared, not copied.
For every State-validated canonical ID, sole saved user ID equality proves the
same exact host, saved ID, native ID and unique-user checks. Every requested
identity is checked in one short read transaction. Any query failure discards
the entire result. Branch metadata conflicts remain eligible and stay saved.
No schema, migrations, native validation, DTO, cap or UI behavior changed.

State uses `try_lock` for the identity reader, so another identity read cannot
create a waiting queue. SQLite busy waiting is zero. A single 100ms deadline
covers the whole SQL batch; the existing pinned rusqlite dependency enables
only its `hooks` feature for the safe progress handler, checked every 100 VM
operations. Deadline checks also surround individual queries and commit. The
handler is removed on every exit. This is a cooperative query-work deadline,
not a wall-clock guarantee for operating-system file opening or I/O.

A missing reader, contention, SQLite error or expired budget passes empty
eligibility to the existing observer. It clears this view's previous targets
and returns Unknown rather than retaining a previous Running claim. Opening
and query failures drop the reader so the next poll retries. The reader guard
and snapshot are released before any observer file or socket work. Closing is
checked before and after acquiring the reader and around status publication.
The existing observer still rejects released, expired or replaced reads.

The bounded primary-mutex regression first failed on the old path with its
expected one-second channel timeout. It always released the held mutex and
joined its reader thread before asserting. It now passes with both exact
eligible Claude and Codex targets while the primary mutex stays held. This
checks target acceptance, not a real Running claim.

Local checks passed with offline dependencies and four Cargo jobs:

- 7 State live-status tests; the manual Claude test remains ignored.
- All 195 Store tests, including 8 new identity-reader tests.
- 57 existing observer tests; the real Codex socket test remains ignored.
- 9 fixture-mode integration tests.
- 8 focused shutdown tests and 28 DTO tests (some overlap earlier suites).

The new Store tests cover one consistent snapshot across a concurrent commit,
fresh committed WAL reads on the next request, exact/unique user identity,
preserved conflicts, missing database refusal, no migration or writes, zero
SQLite busy waiting, partial-result discard, progress-handler interruption and
cleanup, and retry. Synthetic SQL views make errors and costly work explicit;
they exist only in disposable test databases. A synthetic Codex socket peer
establishes Running through the production State/observer path, then reader
contention, SQL error, budget expiry and opening failure each return Unknown,
remove the previous target, and accept it again on the next poll. This is not
proof of a real Codex producer. The socket tests require local socket access;
the sandbox-only attempt was blocked before the peer could bind.

The existing ignored manual Claude acceptance test now holds the primary
database mutex throughout its real Running-to-Idle observation. Its guard is
declared after the cleanup guard and released before shutdown, including on
unwind. It has not been run for this follow-up.

No installed-app changes, live database writes, shared branch movement,
publication or new observer test API occurred.

## Historical UI acceptance

Branch: `feat/claude-live-session-status`. UI base: `ba12f00` (already includes
`integration/current-app`). This document records UI implementation and tests,
not installation approval or proof of the native Claude adapter.

## Changes

Dashboard returned Claude rows use the existing rotating arc only when their
own exact ID has status `running`. The existing Claude SVG is inside it.
Codex keeps its existing logo and behavior. The source tooltip and accessible
name identify Claude Code or Codex. Non-running states keep textual badges.
Sessions uses the same textual badge behavior as Codex, including its recent
activity section. The component defaults to Codex when no host is passed.
Textual badges have named status roles with live announcements disabled.

The eight displayed recent chats retain priority, considering both supported
hosts. Visible table or Dashboard rows share the existing 16-ID cap. Cursor
and unknown hosts receive no live badge or live request. Absent Dashboard
parents receive no badge or request; returned parents and children use their
own IDs. Pagination adds requests only when rows become visible. Existing
counts, parent links and title reads are unchanged.

The existing lease hook is unchanged except for UI host labels and a supported
host check. The wire remains `read(ids, viewToken)`, empty native registration,
`release(nativeView)`, and the existing snapshot shape. Status values remain
`running`, `idle`, `waiting_approval`, `waiting_input`, `unknown`. There is no
generic waiting state, process check, timestamp inference, hook installation,
or new DataSource API in this UI change. Native performs identity validation and
Claude file parsing. The agreed native mapping is permission/sandbox requests
to approval, input/dialog requests to input, and unrecognized reasons to Unknown.

## UI files

- `apps/desktop/ui/src/app/LiveSessionBadge.tsx`
- `apps/desktop/ui/src/app/live-session-status.ts`
- `apps/desktop/ui/src/app/live-session-status.test.tsx`
- `apps/desktop/ui/src/app/SessionsPage.tsx`
- `apps/desktop/ui/src/app/SessionsPage.test.tsx`
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.tsx`
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.live.test.tsx`
- `apps/desktop/ui/e2e/running-arc.spec.ts`
- `docs/acceptance/claude-live-status.md`

## Checks

101 tests passed across these five files:

```sh
pnpm --dir apps/desktop/ui test \
  src/app/live-session-status.test.tsx \
  src/app/SessionsPage.test.tsx \
  src/app/dashboard/ActivityLanes.live.test.tsx \
  src/app/session-titles.test.tsx \
  src/app/session-compactions.test.tsx --maxWorkers=2
```

Tests cover mixed-host states, shared caps, recent priority, scroll changes,
pagination, both waiting states, missing/unknown states, fixtures without live
capability, exact parent/child states, lease expiry, serialized reads, late
registration and status cancellation, navigation, titles and compactions.
Two mocked conflicted Claude table rows receive Unknown from the mocked source
and keep their source-conflict labels rather than another row's Running state.
These tests do not prove the real native identity adapter or live local rows.

A separate regression run included `sub-sessions.test.tsx`: 103 passed and one
failed out of 104 tests. The untouched assertion at line 560 expects four
blank cells after host/name, but the existing Compactions column makes five.
The same failure was reproduced loading ActivityLanes directly from `ba12f00`
using a temporary in-memory Vitest loader, with no checkout edits. This assertion
was left unchanged in the initial UI change.

All 16 running-arc browser cases passed after the final UI change:

```sh
E2E_PORT=5198 pnpm --dir apps/desktop/ui exec playwright test \
  e2e/running-arc.spec.ts --workers=2
```

Those cases cover Claude and Codex, Chromium and WebKit, dark and light,
pixel scales 1 and 2, at 1120 × 720. Across 128 rotation measurements the arc
stays 18px with at least 2px vertical clearance in its 22px row, at least 1px
rounded-logo clearance, the existing gray dark/green light colors, and the
existing 1.4s animation. Reduced motion, actual host asset loading, column
alignment and transition to idle also pass. These are synthetic source states.
The browser test adds the live capability only through request interception;
the production fixture source remains unchanged.

Screenshot and measurement attachment paths:
`apps/desktop/ui/test-results` and `apps/desktop/ui/playwright-report`.
The Claude dark Chromium and Claude light WebKit screenshots were opened and
visually checked. Sessions browser geometry and other viewport sizes were not
checked in the initial UI checks.

`pnpm typecheck`, `pnpm lint`, formatting of all nine UI files, and
`git diff --check` passed. Self-review checked the complete UI diff against
the status contract, selection and cleanup behavior, logo rendering, and
title/count/parent code. Independent combined review was still required then.

## Acceptance limits (initial UI checks)

No native build, native tests, real Claude file read, installed-app test,
installation, publication, app writes or messages were performed in the initial
UI checks. The busy-PID/file-exit proof and the two real conflicted Claude rows
were separately reported evidence, not independently rechecked here.
The full UI suite was not run. The one existing sub-session test failure is
not hidden by the successful focused run.

The offline frozen install reused all 376 packages and downloaded none.
No native build outputs were created.

## Approved follow-up: existing column test expectations

The reported baseline probe loaded the same four production UI files from
`ba12f00` with a temporary Vite loader: both selected failures reproduced, with
37 other tests skipped. That loader was deleted. This follow-up did not repeat
that probe.

- `apps/desktop/ui/src/app/sub-sessions.test.tsx` now expects five blank cells
  after host/name for an absent parent. Its existing link, no-lane,
  no-measurement, no-start, description and no-title-read checks remain intact.
- `apps/desktop/ui/src/app/dashboard/DashboardPage.test.tsx` now explicitly
  checks Compactions at header index 2 and PRs, started, activity and output at
  indices 3 through 6. Its test name and comment include Compactions.

No production columns, sources, runtime, native, CSS, assets, settings or wire
types were changed by this follow-up. The earlier unresolved test failure and
full-suite limitation above describe the initial UI checks; this section records
the later approved fixes.

Both complete test files passed: 39 tests, two workers. The full UI run then
passed all 834 tests in all 87 files, with two workers, in 52.20 seconds:

```sh
pnpm --dir apps/desktop/ui test --maxWorkers=2
```

Formatting of both test files and this document, and `git diff --check`, passed.
Self-review confirmed that only the old column expectations, their test name
and comment, and this acceptance document changed in this test-only follow-up.
Production behavior and every other assertion in both test files remain unchanged.

No test process from this follow-up remains running.
Independent combined review was still pending then;
this full UI pass is not native or installed-app verification.
