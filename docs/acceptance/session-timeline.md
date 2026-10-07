# Session detail timeline acceptance

One session's page shows the M-09 hands-off stretches it contributed in the
selected window, above its transcript, and lets a reader go from a stretch to
the tool call it started with. Each stretch also says what M-20 measured about
it: its active time, how many of its calls repeated an earlier one, which tool
repeated most, and whether it was circling. The stretches come from
[session-stretches.md](session-stretches.md); the transcript from
[session-source.md](session-source.md).

Run `pnpm --dir apps/desktop/ui exec vitest run src/app/session-detail
src/data/DataSource.test.ts` and `pnpm check`.

## The query

- `sessionStretches(sessionId, windowDays)` is a metadata query, cached under
  the `sessions` prefix beside the session's row and invalidated with it by
  every ingest event.
- Changing the range re-reads the stretches for the new window and **never**
  re-reads the transcript, whose read is keyed by the session alone. The
  session page owns the range control, because its row and its stretches are
  both measured over the window.
- The same answer carries M-20. The app reads M-09's stretches and M-20's
  repeats **inside one database snapshot over the same selected window**, so
  the stretches M-20 counted are the stretches M-09 states even while the
  index is being written. Each report crosses through a conversion that
  refuses any change from the metric's own shape, and the two are set side by
  side only when they agree — the same state, the same excluded surface, and
  for every stretch the same endpoints and elapsed duration. Two reports that
  disagree are an error, never a best guess. M-09's fields are unchanged, and
  a test holds them equal to M-09's own report and M-20's fields equal to
  M-20's.
- M-20 uses its default thresholds, four active minutes and five repeats, and
  the answer states the thresholds it was judged against. There is no setting
  for them yet.
- Native, fixture and preview answer one contract: the native command; the F1
  export's entry for exactly that session and window, `missing` for an
  identity it does not list, and a refusal for an unsupported window; and a
  refusal in browser preview.

## What is shown

- **Loading**, **failed** (with a retry), **missing**, **unmeasured** and
  **measured with no stretch** are five different sentences, and none is
  worded as another. An unmeasured session names its excluded surface and that
  surface's counts when that is the reason, and says it is about how the
  surface records time; it never claims the session's own timestamps are
  absent.
- A measured stretch is a native `<button>` in the order M-09 stated, never
  re-sorted. It shows when it started, in the reader's zone, and how long it
  lasted **from `duration_ms` only** — never from its endpoints — with a bar
  sized by its share of the longest stretch and no time axis. Its accessible
  name states its position, start, duration and whether its first call can be
  shown, and why not when it cannot. `aria-pressed` marks the selected one.
- A stretch whose first call cannot be shown is still drawn, still a button,
  and still selectable: it was measured whatever the transcript says. When one
  is selected, **the reason is on the screen** as well as announced — a reason
  carried only in the segment's accessible name is a reason a sighted reader
  never receives. One polite live region holds it, stays in the document with
  nothing to say so it is already watched when it speaks, and takes no room
  while it is empty.
- The measurements above and the stretches stay on the page when the
  transcript cannot be read.

## Repeated calls (M-20)

- **What a repeat is.** Inside one stretch, calls to the same tool with the
  same command, path or pattern form a group; each call after the first in its
  group is one repeat. The grouping is done when the call is indexed, and only
  an opaque comparison key is kept. **No key, digest or argument is part of
  the answer**, is shown, or is logged; a test asserts none crosses.
- **Active time is not length.** Under a stretch's length (M-09's elapsed
  time) it says its active time, labelled as active. Active time is the agent
  minutes rule (M-05) over the stretch's own records, so a long wait inside a
  stretch counts for none of it: a stretch can last an hour and be active for
  one minute. Active time can be zero, and is then said as `0h00m`. The
  length is hands-off time, written as every hands-off median is (`61 min`,
  read aloud `61 minutes`); the active time is agent time, written as all agent
  time is (`0h05m`, read aloud `5 minutes`).
- **A count, or an honest unknown.** A measured stretch says `no repeats` or
  how many, and names the tool that repeated most **by its name and count
  only** — `Edit ×9`. A stretch whose calls could not all be counted says
  `repeats unknown`, never `0`, and its accessible name and the selected
  stretch's note say why, in plain words: a turn that did not record its call
  count, calls missing from the index, calls **indexed before this app
  compared calls** (older indexes are not re-read, so those stay unknown
  until the calls are written again) or whose arguments could not be compared,
  a comparison made by another version, or two readings that disagree about a
  call.
- **Only circling is set apart.** A stretch the metric judged circling has a
  warning-coloured border and background **and** a `Circling` tag, so colour is
  never the only sign. A stretch judged not circling is drawn normally, and so
  is one whose repeats were unknown: its accessible name says whether it was
  circling is not known, never that it was not. The view does not recompute
  the judgement from the numbers.
- **The rule is stated.** Under the stretches, one sentence explains circling
  from the thresholds the answer carried, says active time leaves out long
  waits, and says it describes the calls and does not say the agent did
  anything wrong.
- **Order is M-09's.** Nothing is ranked or re-sorted by repeats.

### Showing a repeated call

- When the selected stretch names a most-repeated call, a native `Show
repeated call` button appears under the stretches. It goes to the
  **earliest** call of that group, through exactly the path a first call
  takes: the same position check against the loaded transcript, the same
  reasons when it cannot, and the same reveal request. Nothing new is looked
  up, and there is no fallback.
- Its accessible name says why the call cannot be shown before it is pressed;
  pressed, the reason is on screen. Focus stays on the button, the stretch
  stays selected, and the live region says `Showing the earliest of the
repeated calls in stretch N: Tool.`
- When the repeated call is the call the stretch started with, pressing it
  reveals that call again as a new request.
- The press belongs to the same session, window, transcript read and accepted
  answer as a stretch press, and is destroyed with any of them.
- A stretch with no repeats, or whose repeats were not counted, offers no
  button and says which.

## Resolving a stretch's first call

`{record_uuid, block_index}` is matched against the session's structured
payload as it was loaded for this open, and nothing else:

- The record identity is compared as the bytes it was recorded with, and the
  index as a number. A target is **exactly one block at that position, and it
  is a tool call**.
- Each other outcome is its own reason and no jump: no locator (M-09 could not
  establish the first call), the transcript still loading, the transcript
  unavailable or failed, nothing at that position, more than one block at that
  position (a fork can carry two records under one UUID), and one block that
  is not a tool call.
- There is **no fallback** to the host's call id, to the nearest call, or to
  the record the call would have been in.

## Revealing it

The transcript takes one small explicit request — a token and the composed
block id — through `SessionTranscript`'s `reveal` and `onReveal`:

- The one tool row holding that block opens itself **only if it is closed**.
  After the commit that mounts its cards, it takes the card at that block's
  position from its own refs; the anchor that stood there is gone by then.
  **Nothing is looked up in the document**: no selector is built from an
  identifier, and a test asserts no query during a reveal names the block.
- The card is scrolled into view within its scroll container — smoothly, or
  instantly when the reader asked for reduced motion — and outlined. Focus is
  never moved: it stays on the segment the reader pressed, and a polite live
  region announces what happened, or why nothing did.
- Each request is answered once. A request made against one session, one
  window or one read of the transcript is not handed to the transcript once
  any of those changes, so a pending jump is cancelled by session change, range
  change, read replacement and unmount, and a press made while the transcript
  was loading does not fire later. A late report for a replaced request matches
  nothing.
- **A press is destroyed, not hidden.** It means "the stretch at this index, in
  this list", so it belongs to one session, one window, one read **and one
  accepted stretches response**. A replacement of any of them clears it: during
  render so no frame of it is painted, and in an effect so it cannot return.
  Merely hiding it lets 7d → 30d → 7d bring back an index nobody selected, and
  lets a refetch that reorders the same window's list leave the press pointing
  at a different stretch. Both are covered by regressions.

## Verified in WebKit

Against a private preview in the cached WebKit build, offline, with the target
card sixty turns down: Enter and Space on a focused segment revealed it; the
content area scrolled to bring it into view; it carried the outline; focus
stayed on the segment with `aria-pressed="true"`; the live region announced the
tool; pressing another segment moved the highlight; a stretch with no locator
highlighted nothing and announced why. Both motion settings, no page or console
error.

### Verified in WebKit

Against a private preview in the cached WebKit build, offline, in the light and
dark themes at 1280×800 and 1024×720: only the circling stretch carried the
warning border, fill and `Circling` tag; nothing overflowed its segment or the
page; Enter on the circling segment revealed its first call, one Tab reached
`Show repeated call`, and Space revealed the earliest repeated call in view,
with focus kept on the button and the live region saying so; the unknown
stretch explained why and offered no button. No page or console error. The
check found the repeat line cut off at both widths, which is why it now wraps.

## Not in this stage

- **No argument labels.** The most repeated call is named by its tool and
  count only; the command, path or pattern it repeated is not read from the
  transcript to label it.
- **No settings** for M-20's thresholds, and no persistence of them.
- **No backfill.** Calls indexed before comparison keys existed stay unknown
  until they are written again; nothing re-reads a source to fill them.
- **No ranking or shading by severity**, and no window-level or dashboard-level
  M-20 figure. Only the circling stretches are set apart. SPEC U-09's ranking
  and shading by M-20 is still future work; chronological order is the
  approved contract for this slice.
- **M-20 is not in the reviewed definition catalog.** The section's definition
  control — the same compact info control as the Dashboard's cards, named
  "Hands-off stretches definition" — shows M-09's definition, and the rule ID
  appears only inside it; no metric ID is visible on the page. M-20's rule is
  stated in the section's own plain sentence, from the thresholds the answer
  carries, until the catalog is reviewed and extended.
- **Not yet verified end to end against a native index with reader
  integration.** The native command, its fixture parity and the page are
  tested; the combined native run is later work.
- An unmeasured session with no excluded surface names both of the core's
  reasons, because the core does not say which applied: a boundary it could not
  place, or a segment whose tool use it could not establish.
- A result strip is captioned with the call's verb, as elsewhere in the
  transcript.
- A start label is shortened with an ellipsis when its segment is narrow; the
  full time is in the segment's accessible name. The repeat line wraps instead,
  so the most repeated tool and its count are never cut off.
