# Automatic parent links for Codex-launched Claude sessions

When a Codex agent starts a Claude Code worker with an explicit session ID,
the worker's conversation is listed under the Codex conversation that
started it — whether the worker's job later succeeds, fails or is still
running — in the Dashboard's Sessions table and on the worker's own page.
The Sessions list keeps every worker as its own searchable row. This happens
automatically, for workers launched before or after the app starts, with no
hook, wrapper or setup.

## What counts as a launch

A launch is one operation of a Codex agent's own `exec` code cell, read as
data (nothing is run):

- the cell is string constants and one or more awaited
  `tools.<name>(...)` operations, each emitted once by `text` right after
  it; `exec_command` / `write_stdin` argument values are literals,
  constants, `+` joins and the `.replace(/'/g, …)` quoting idiom only;
- any other tool (a file edit, for example) is an opaque operation: it keeps
  its place among the operations and results, so it never shifts a launch
  onto another operation's result, and its arguments are never searched for
  a launch. They must be plain data — the values above, `null`, JSON
  numbers (`-1`, `1.5`, `-0`, `1e-3`; never calculated), template literals
  without `${…}`, and objects and arrays of these, at most 16 levels
  counting the argument itself as the first; a call, a result, a
  substitution, arithmetic or a non-JSON number (`01`, `0x1F`, `1n`, `NaN`,
  `Infinity`) refuses the cell. `exec_command` / `write_stdin` numbers stay
  unsigned integers;
- the operation's command is one literal Claude print command, optionally
  led by literal environment assignments (`NAME=value`, a valid unquoted
  name, a literal value, set aside; any expansion refuses): the bare
  `claude` program, or an installed Claude launcher by absolute path
  (`~/.local/bin/claude`, `~/.claude/local/claude`,
  `/opt/homebrew/bin/claude`, `/usr/local/bin/claude`,
  `~/.npm-global/bin/claude`, `~/.bun/bin/claude`, `~/.volta/bin/claude`,
  present now as an executable). Its session is named by `--session-id`
  once with a lowercase UUID (literal, or loaded from the immediately
  preceding own cell's one-use printed binding). Its prompt is one literal
  inline word or one `< /absolute/path` (never opened; then no prompt is
  compared), never both. Optionally one `> /absolute/path` and one
  `2> /absolute/path` or `2>&1` (never opened; no `/dev` path);
- its options are read through the one shared Claude option map
  (`claude_launch/options.rs`), which the Claude `Bash` launch reader also
  uses. It lists each known option once with its spellings, how many words
  it takes and what it means; options may come in any order, and a long
  option's value may follow `=`. `-p`/`--print` once is required. Neutral
  options (model, fallback model, effort, permission mode, name/`-n`,
  system prompt additions, settings, tools, allowed/disallowed tools, added
  directories and the permission, safe-mode, slash-command, strict-MCP and
  browser switches), `--max-turns` (positive integer) and `--max-budget-usd`
  never decide which session is created. Neutral does not mean every value
  is valid: a launch Claude refuses before starting any session is no
  launch, so a later valid retry of the same ID is the only one. So
  `--mcp-config` is read only as the literal empty-server configuration
  (`{"mcpServers":{}}`, with only JSON whitespace — space, tab, carriage
  return, line feed — around its tokens, matched byte by byte in place with
  no JSON decoding; `{}` is refused, and an escaped or duplicate key,
  another member, a non-empty value, trailing data, a file or any other
  configuration is never opened and is unsupported);
  `--max-budget-usd` only as a plain positive decimal (`5`, `0.5`; zero,
  words, signs and exponents are not read); and `--output-format
stream-json` with `--print` only together with `--verbose`.
  `--output-format` `text`, `json` or `stream-json` (with `--verbose`) and
  `--verbose` otherwise change only what is printed, which this proof never
  reads. `--tools`, `--allowedTools`, `--disallowedTools`,
  `--add-dir` and `--mcp-config` take exactly one value here and are never
  followed by a plain word (the CLI would take it as another value);
- the host acknowledged that operation: its own first result — at the
  operation's position among the results its cell emitted, in the launch
  call's output or in the output of a `wait` on the cell it announced
  running — is a process result, an object with a string `output` and either
  an integer `exit_code` (any code: the command ran) or a numeric
  `session_id` (the process is running). It is taken as soon as it is
  emitted, even while the cell still runs, and nothing after it is read for
  the launch: no later poll, exit, job failure, input, note or host event is
  needed or can undo it, and a handle number used again is not an identity.
  A tool error, an approval denial, any other result in its place, another
  operation's result, another call's output, a missing, repeated or
  unreadable output of the launch or its waits, an unreadable `wait` on its
  cell, or a completed cell whose results are not one per operation leaves
  the launch unacknowledged;
- a running cell belongs to the one exec call whose output announced it
  (until a `wait` on it sees it complete): operations of one call share its
  cell, its waits and their positions, but a cell two exec calls claim at
  once ends every launch waiting on it;
- launch and acknowledgment lie in one physical history file, in rows that
  are the thread's own (not below a spawned thread's
  `subagent_history_start_ordinal`);
- an older native tool output the Codex app wrote without a call identifier
  (its own non-blank native `id` and an output value, as the reviewed reader
  accepts it) pairs with nothing: its row is still checked (JSON, types,
  ordinal), it is not counted and its own `id`, name or body is never taken
  for a call identifier; it ends every launch not yet acknowledged when it
  appears, and a cell it announces running is unusable in that file. Any
  other tool row without a call identifier refuses the history.

Refused, as the map reads them: resume (`--resume`/`-r`, `--from-pr`,
`--teleport`) and `--continue`/`-c` (an existing session), `--fork-session`
(copied history; not a fresh child), `--bg`/`--background` (alone it returns
at once; with resume it may continue or copy a session — conditional),
`--no-session-persistence` (nothing saved), `--help`, `--version` and a
prompt word that is a CLI subcommand (no session), no `-p` or
`--input-format stream-json` (no single print turn), streaming output
without `--verbose`, an invalid MCP configuration or budget (refused by
Claude before any session); also any option not in
the map (its arity is unknown, so the prompt cannot be found), a repeated
option or alias, a malformed value, `--`, a prompt both inline and
redirected, pipes, lists, substitution, background jobs, templates, any
other program path or redirection (`>>`, `2>>`, `&>`, `2>/path` without a
space), and an assignment that is quoted, expands, is an array or `+=`, or
follows the program. A session ID mentioned in prose, a quoted script or a
tool output relates nothing.

## What the child must be

Exactly one indexed Claude user session with that ID, fresh: its one
recorded transcript is its own (every line names it, none dated before the
launch), its first eligible input (a user record that is not meta, sidechain
or a tool result, with text or not) is one text exactly equal to the prompt,
dated at or after the launch, appearing once, the unique first input (no
other input untimed or dated at or before it, as the store's creation rule
requires), and no other project holds a file of its name. The first input
may come any time after the launch — before or after the acknowledgment or
the job's end; nothing in the parent's history bounds it. Later inputs are
allowed: the conversation counts as agent-created from its first input. A later
`--resume` of the same session is not a creation and changes nothing. A
transcript whose last line has no newline yet decides nothing: the launch is
retried once the transcript changes, and the transcript is not read again
before. Right before the link the transcript must still be the generation its
first input was read from; a changed one is checked again from its new
content. A projects listing that fails now is retried, not a rejection.

## Codex sessions started with `codex exec --json`

The same scan also lists a Codex conversation under the Codex conversation
whose agent started it with a literal fresh `codex exec --json` run. The
command never names the new conversation: its saved ID is the one
`thread.started` event the run printed first, read from that launch
operation's own first process result — in the launch call's output, or in a
`wait` on its running cell — even while the run goes on. A result that names
no thread yet, or a run that failed before one started, is no launch; a
later `write_stdin` poll is never read for one.

The one maintained map of `codex exec` options (`codex_cli.rs`, checked
against `codex exec --help` of Codex CLI 0.157.0) reads each option's
spellings and argument count in any order, with Codex's meanings: `-c` is a
configuration override and `-p` a profile, each taking one value. Only the
bare program `codex`, `exec` or its alias `e`, `--json` and at most one
prompt word is a launch; standard input and standard error may be
redirected (never opened), standard output and leading assignments may not.
Resume, fork and review (at the top level or under `exec`), `--ephemeral`
(it saves no conversation), help and version are each their own action and
never a fresh start; a run without `--json` prints its ID only as text,
which is never read; an option the map does not describe (`-i`,
`--thread-source`, a top-level option) or `--` leaves the command
undecided. An ID mentioned in a prompt, a title or any other output relates
nothing. Only output lines that start as a JSON object are events, and
nothing nested inside one is read. The first event must be one complete,
valid `thread.started`; it names the child. Each later event is read only
for its top-level `type`: once that type is decoded as anything else, a cut
or malformed rest of the line (an output cut short) is ignored. A later
event whose type is `thread.started` (whatever its ID, complete or not), or
whose type is missing, cut, not text, given twice or after a payload that
breaks before it, names no thread; so does output without a complete first
start.

When one cell starts several runs, each launch's own result is found by
replaying that launch's acknowledgment follower over the saved rows from the
launch row to its acknowledgment row, as the scan did: the result at that
operation's own position among everything its cell emitted, whether it came
in the launch's output or in a `wait`, and only that result.

The child must be exactly one indexed Codex user session whose one recorded
original rollout, inside the Codex history root, opens with its own header:
`source` exactly `exec`, opened after the launch, naming no parent, fork,
history base or inherited rows. Only that opening line is read; the file
must be the same generation after it, and again right before the write. Its
first input is the index's own first eligible input, which must be dated at
or after the launch; until the index holds one the launch waits. The child
fact (`codex_cli_launch`) needs no parent; the relation follows the same
store checks as a Claude child (exact canonical parent, no self link, no
cycle, no reuse, one child per launch, competing anchors withheld). A thread
whose own launch names itself, and a launch made after the child opened,
are refused. Claude `Bash` callers of `codex exec`, launcher paths and
native `exec_command` function calls outside an `exec` cell are not read in
this version.

## Which history is the parent's

The index records only a Codex thread's original rollout. Its continuations
(`rollout-<ts>-<thread>_<rollout>.jsonl`) are found by one name-only census
of the Codex history root, kept across passes until complete; no group is
taken from an incomplete census. The census and the check that no other
project holds a child's file look at directory entries one at a time within
the pass's budget and keep their place. The group is validated from each
file's opening header as the reviewed reader's `codex_history.plan` does: one
original, unique rollout IDs, bases in the group without a cycle, each
header's ordinal equal to its base's `end_ordinal_exclusive`, each base
cutoff a line boundary whose row has the ordinal just before. Every row read
is decoded for its structure; paginated rows must carry consecutive actual
ordinals, and no `session_meta` may follow the first line. A fork, an alias,
a file outside the root or an indexed locator the census did not find
refuses the thread.

## One validation per history

A thread's whole history is validated at once and published as one
revision: the exact member files with their generations (device, inode,
length, modification and change times), a `valid` or `invalid` verdict, and
the launches found. Before any slower work a replacement validation marks the
thread `pending`, and nothing of a pending thread is linked. Every member is
read whole; every call identifier's occurrences are counted, calls and
outputs apart, and a launch is kept only if each identifier followed to its
acknowledgment — the launch call and any `wait` on its cell — occurs exactly
once as a call and at most once as an output in the whole history, whichever
file it is in (so a repeated launch output anywhere refuses it). The
acknowledgment itself is matched on exact identifiers; the counts use a keyed
hash, so a collision can only reject. A malformed member makes the whole history
`invalid`; a member without launches is an ordinary valid member. Each file
is read only as the generation its header was read from when the history was
planned (a base cutoff is checked in that generation too); any other
generation plans the history again. A history refused for what was read is
published `invalid` only if every file read is still that generation;
otherwise it is planned again. A member whose last line has no newline
yet holds the thread `pending`: nothing is published or linked, and it is not
read again until one of its files changes.

Launches are staged apart and published with the member set in one
transaction, only by the latest attempt and only from the revision it started
from. A link is written only if, right before it, the census is current and
finds exactly the published members, each exactly its generation, and the
store's guarded write finds the published revision, members and launch still
exactly as checked. A history that is unchanged and validated is never read
again, even after a restart; an unfinished validation is not stored, so after
a restart it reads every member again. Within a run, members already read are
kept in memory, so only a changed member is read again. A child imported
after its launch was passed over has the launch looked at again before the
thread's launches are done, even while another check is under way.

## Versions

A published validation records the scan's validation version (9). A
history validated by another version — such as version 8, which did not look
for `codex exec --json` launches, or version 6, which refused launches for
their output format, verbose logging or standard-error routing — is read
again once, then not again while unchanged. A Codex child's fact and
relation carry `codex_cli_launch` version 1. New proofs and child
facts carry creation evidence version 6, the only one the store accepts for
new writes. A relation or child fact an earlier version accepted stays
exactly as stored: a proof with the same creation anchor (the same child,
parent, first input, launch operation, history file and launch row) replays
it, as does one for a child whose accepted foreground CLI relation names the
same child, parent, first input and launch operation; the launch is marked
linked and nothing is rewritten. A genuinely different anchor withholds the
child as before. Claude `Bash` launch child facts carry version 2 from the
same map; the `Bash` reader recognizes verbose launches (streaming JSON
among them) but never reads what they printed as an answer, so they decide
no child.

## Bounds

- Each watcher pass shares one budget (the bytes and time its spawn work left
  of 256 MiB and 5 s, 100,000 directory entries, and the cancel) across the
  census, opening lines, base cutoffs, members, recorded lines, child
  transcripts and project listings. Every read asks the budget first and is
  charged what it read; a read longer than a pass keeps its partial buffer
  for the next.
- Memory: at most 2 threads hold work at once, each within 112 MiB of
  retained buffers (partial lines, identifiers, running cells, launches
  found and their staged copies, a check's copies, prompts,
  child-transcript state; a launch waiting for its result keeps a count of
  the results before it, never the results), plus 24 MiB of idle member facts and 8 MiB for the
  pass's read chunk and store pages: 256 MiB in all. Buffers are reserved at
  their actual capacity before they grow; decoding a row reserves a bound
  computed from its structure (tested against the allocations it makes). A
  third thread waits its turn. Separately, 4,000,000 call-count and cell
  entries in all (1.5 M per thread, 1 M cached), 1,000,000
  followed-identifier references and 100,000 launches per history, 64 open
  launches per file, each followed at most 64 MiB or 200,000 rows to its
  result, 32 operations per cell, rows up to 16 MiB, 2,000,000 census
  entries.
- Only input alone over its own limit is refused (a launch unacknowledged, a
  member or history `invalid`); other work filling memory makes a thread wait, never
  its input invalid.
- A thread that pauses, waits or meets a busy source goes behind the others.
  A census that could not read an entry is taken again later; a child not
  indexed waits for its import or the next start; a busy source is retried on
  the watcher's backoff. A store error keeps every queued thread and child,
  and links committed before it in the pass are still counted and announced.

## What is stored

Schema 15 adds the evidence kind `codex_claude_launch` to
`session_creation_relations` (rebuilt with its rows unchanged) with the
parent's exact canonical and native IDs, the child's first input record,
the launch call ID and operation position, the acknowledgment's call ID,
operation position and row ordinal (in the `completion_*` columns; from
version 3 they locate the acknowledgment, in version 1 the exit 0), the
process handle when the result had one, the history file's rollout ID and
the launch row's ordinal; and
four tables: validations (revision, last revision issued, status), their
member files and generations, published launches (with the revision that
found them, a parent-side verdict and a child-side state), and staged
launches. No path, prompt, command, output, time or digest is stored. The
store re-checks the child, its first input, the exact parent, reuse and
cycles; a second child claimed for one launch, or a different claim on a
child, by either Claude CLI kind, withholds every contradicting relation.

Schema 20 adds the evidence kind `codex_cli_launch`, with the witness
`codex_exec_json_thread_started`, to `session_creation_relations` and
`session_child_facts` (both rebuilt with their rows, row IDs, indexes and
triggers unchanged): the same identifiers as `codex_claude_launch`, with a
Codex child. Published and staged launches gain the child's host, `claude`
for every row stored before.

Like every accepted creation relation, a link makes the Human input view
count the child's user messages as the agent's, not a person's.

## Checks

- `cargo test -p xt-store --test claude_launch_creation`
- `cargo test -p xt-ingest --lib claude_launch` (includes read accounting,
  injected store failures and a forced hash collision)
- `cargo test -p xt-ingest --test claude_launch_links`
- `cargo test -p xt-ingest --test claude_bash_children`
- `cargo test -p xt-store --test child_facts`
- `cargo test -p xt-store --test codex_cli_launch_creation`
- `cargo test -p xt-ingest --test codex_cli_launch_links`

All sources are synthetic. Acceptance on a copied index uses
`cargo run -p xt-ingest --example claude_launch_links` (see its header); with
`--backlog-only` it stops after 20 passes in a row that move nothing (only
unchanged unfinished files or timed retries left), as the watcher would back
off, and reports `threads_unfinished`, `children_unfinished` and directory
entries per pass.
