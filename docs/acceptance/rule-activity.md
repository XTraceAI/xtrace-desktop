# Recorded rule activity transport

The app reads recorded rule activity on demand from one source: the default
local rulebook, `~/.config/memhub-plugin/rulebook` under the native home
startup selected. One read is one bounded snapshot of its ledger
(`ledger/schema_version` must be `2`, rows from `ledger/fires.jsonl`) over the
fixed trailing 14 × 24 hours, returned as structural metadata. This is the
transport only: no page reads it yet, and the Dashboard's rule tile is
unchanged.

## What a read can say

- **Recorded rows in one source, not activity.** The plugin can fail to
  record, and the ledger promises no retention, so an empty snapshot is
  "0 recorded rows", never "no rules fired", and the oldest observed row is not
  a coverage start. Other rulebook roots are not read.
- **Modes, not outcomes.** `gate` does not mean blocked, `advise` does not mean
  followed and `suppressed` was not delivered. An unrecognized mode keeps its
  recorded value and is counted apart.
- **Precision.** `exact` only for a complete, clean scan of the whole ledger;
  any byte or line bound, malformed row, conflict or unfinished line makes
  every count, and every group's, a `lower_bound`. A presentation bound never
  lowers precision.
- **Two presentation bounds.** At most 100 latest rows (`fires_truncated`) and
  100 observed groups (`groups_truncated`). Groups and counts cover every
  accepted in-window row, not only the returned ones; `observed_group_count`
  counts them all. A group is an observed `(rulebook, rule)` pair, never an
  active or distinct rule; a row without a rulebook is an unscoped group.
- **Nothing else crosses.** No path or root, file identity, raw JSON, excerpt,
  override reason, dedup key, source message ID, matcher count or parser and
  filesystem text. The recorded agent, worktree, repository and branch stay
  native-only. Session identities are raw and not linked to indexed sessions.

## Admission, cancellation and quitting

- The source path is computed once at startup from the native home; nothing is
  read, checked or created until a view asks. Fixture mode has no native home
  and answers `unavailable`/`not_configured`; it never falls back to the
  user's home.
- One read runs at a time. Admission takes the one slot before any source
  access and before a worker is spawned, so a refused read answers `busy` at
  once and never waits in the worker pool. A duplicate identifier is `busy`
  too; the running read is never shared or replaced.
- The read runs on the blocking pool and holds no database lock. The slot is
  released only when the read returns, on every path including a panic.
- Cancellation is cooperative: a cancel before its read is remembered (the 64
  most recent), one during it cancels that read alone, and a read cancelled or
  closed after the source was read still returns no snapshot. A cancel never
  frees the slot early.
- Quitting refuses new reads, cancels the running one and waits at most three
  seconds, before the index and database close. Neither this nor the reader's
  one-second deadline can preempt a blocked OS call; no hard wall-clock bound
  is claimed.

## Verification

All sources are synthetic temporary homes; no real ledger is read.

- `cargo test -p xtrace-desktop --lib rule_activity` — exact wire shape, the
  anchored window and its half-open bounds, privacy of every state, exact
  versus lower bound and both truncations, busy/duplicate admission, no queued
  admission, pre/during/after cancellation, cancellation and close after the
  source read, panic and error release, bounded close, not-configured and
  construction without reads, and a read answering while another thread holds
  the database lock.
- `cargo test -p xtrace-desktop --lib rule_activity_commands` — both commands
  through Tauri's IPC dispatch: `readId` only, typed states, an extra root
  argument ignored, closed after close or with no service.
- `cargo test -p xtrace-desktop --all-features --test rule_activity --test fixture_mode`
  — repeated reads leave source bytes, modification times and
  the app database unchanged; a missing source creates nothing; fixture
  startup has no source; the F1 export's `rule_activity` matches the running
  fixture app.
- `scripts/ci/check-dto.sh` and `DataSource.test.ts` — generated types match
  the Rust DTOs; native, fixture and preview transports.

Not established here: any page, Dashboard metric, session linking, rule text,
active rules, outcomes, native install behaviour or a read of a real ledger.
