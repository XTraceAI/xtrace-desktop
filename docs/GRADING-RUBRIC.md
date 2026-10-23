# Grading rubric (`xtrace-rubric.md`)

A skill joins XTrace's Scores page by having a rubric: a small Markdown file
named `xtrace-rubric.md` in the skill's folder, next to its `SKILL.md`. The
skill itself stays unchanged. XTrace runs the skill on a project with Claude
Code, then, in the same run, has Claude read the rubric, judge the skill's own
output against each criterion, and write XTrace's `result.json`.

## Example

```markdown
---
protocol: 1
id: readme-check
name: README Check
version: 1.0
description: Checks that a project's README explains how to install and run it.
target: project
---

Optional notes for whoever reads the rubric.

## Install steps (60 points)

Award 60 when the skill found working install steps, 30 when they are
incomplete, 0 when there are none.

## Run steps (40 points)

Award 40 when the skill found a command that starts the project, else 0.

## Findings

One finding per problem the skill reported; use its suggested change as the fix.
```

## Rules

- **Header.** The file starts with a `---` line and the header ends at the
  next `---` line. In between, one `key: value` per line with exactly these
  keys, each once:
  - `protocol`: `1`.
  - `id`: lowercase letters, digits and `-`, starting and ending with a letter
    or digit, at most 64 characters. Stored grades are matched by this id.
  - `name`: the name shown on the Scores page.
  - `version`: no spaces. XTrace stores each grade with this version plus a
    short hash of the rubric file (for example `1.0 #3fa2b9c1d0e4`), so an
    edited rubric is visible even when the version was not changed.
  - `description`: one line saying what the skill checks.
  - `target`: `project` (a run examines one project folder).

  A value is the plain text after the colon; quotes are kept as written.

- **Criteria.** Each `## <name> (<N> points)` heading (or `(1 point)`) is one
  criterion, with plain-language guidance below it. Points are whole numbers
  from 1 to 100 and add up to exactly 100. A rubric has 1 to 20 criteria, and
  no two share a name (ignoring case). A criterion's score is a whole number
  from 0 to its points; the grade is their sum.
- **Findings.** An optional `## Findings` section (no points) says how to turn
  the skill's output into findings: severity (`critical`, `warning`, `info` or
  `pass`), title, detail and fix.
- Text between the header and the first `##` heading is free. `##` lines
  inside fenced code blocks are part of the guidance. A fence opens with at
  least three backticks or tildes indented at most three spaces, and closes
  only with at least as many of the same character and nothing after them.
  `## Findings` with points is refused. The file is UTF-8 and at most 64 KiB.

A rubric that breaks a rule is refused with a typed reason, and the skill does
not run. The reader is `crates/xt-judge/src/rubric.rs`.

## Adding an installed skill

The "+" card on the Scores page lists the skills installed on this Mac:
`~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills` and the enabled,
user-scoped Claude Code plugins. A skill whose folder has a valid
`xtrace-rubric.md` can be added; it then has its own card, keyed by the
rubric's `id`. A skill whose rubric `id` (or, without a rubric, whose
`SKILL.md` name) is already on Scores cannot be added again, and of two
installed skills with the same rubric `id` only one can be added. Removing a
skill from Scores keeps its stored grades and its files. If an added skill's
folder or rubric later goes away or stops being valid, its card stays and says
why, and nothing starts. A run copies the skill's folder (at most 2,000 files
and 50 MB, hidden entries skipped; a link may only name a file inside the
folder).

## Adding from a link

The top of the Add a skill dialog has **Add from a link**: paste a public
GitHub link and press **Look up**. Accepted links are
`https://github.com/<owner>/<repo>` (with or without `.git`),
`…/tree/<branch, tag or commit>` with or without a folder after it, and
`…/blob/<branch, tag or commit>/<folder>/SKILL.md` (that folder), each with
or without a trailing slash and with or without `https://`; a query or
fragment GitHub's pages add (`?tab=readme-ov-file`, `#readme`) is ignored.
Anything else says "Only public GitHub links work for now." A branch whose
name has a `/` cannot be named yet.

- **Network.** Look up is the only time XTrace goes online for this, and
  only to GitHub, without signing in: GitHub's API names the commit the link's
  branch, tag or commit points to now (the default branch when it names
  none), and that commit's archive is downloaded from `codeload.github.com`
  (at most 50 MB). GitHub allows a limited number of unsigned look-ups per
  hour from one network; past that, Look up says to try again later. Cancel
  stops a look-up going.
- **Unpacking.** Only the link's folder is unpacked, into a private staging
  folder in XTrace's data folder, at most 5,000 archive entries and 100 MB.
  An absolute path, `..`, a link out of the repository, a hard link or a
  device refuses the whole download. Nothing downloaded is run, then or when
  it is kept: a kept skill is only stored. Grading and making a rubric run it
  later inside the same sandbox as any skill (project read-only, network off).
- **Preview.** The link's folder is the skill when it has a `SKILL.md`;
  otherwise every folder with a `SKILL.md` up to three levels below it (not
  inside a skill already found, not in `.git`), at most 100. Each shows its
  name and one-line description, its files and size, whether it has a valid
  `xtrace-rubric.md` (the same reader), the repository and short commit, and
  the folder name it would get: its `SKILL.md` name when that is a plain
  skill name (lowercase letters, digits and `-`), otherwise its own folder's
  name. Several skills get checkboxes, none ticked at first. A skill can't be
  kept when that name is already used in `~/.agents/skills` or
  `~/.claude/skills`, when a skill listed above it gets the same name, when
  it is larger than grading copies (2,000 files, 50 MB), when a link in it
  does not name a regular file inside the skill's own folder (each link is
  read from where it sits in the skill, as it will be once kept under its
  new name: a `..` that rises above the skill's folder, a link to a folder
  or to another link is refused), or when its `SKILL.md` can't be read; its
  row says which. Of two skills that get the same folder name, only the one
  listed first can be kept. Renaming is not offered yet.
- **Keep.** XTrace checks the chosen skills again (if a name was taken
  meanwhile, nothing is kept and the preview shows it), copies each into a
  hidden folder in `~/.agents/skills` and renames it to `~/.agents/skills/<name>`,
  never over anything already there. It then links `~/.claude/skills/<name>`
  to it when that name is free there (and `~/.claude` exists); otherwise it
  leaves that name alone and says so. `~/.claude` and `~/.claude/skills` may
  be links to folders you own. Where each kept skill came from
  (repository, folder, the branch or tag asked for, commit, when) is recorded
  in `<XTrace data folder>/linked-skills.json`, not in the skill's folder.
  A records file XTrace cannot read is moved aside to
  `linked-skills.json.bad` (replacing an older one) rather than overwritten.
  If XTrace stops in the middle of keeping, its next start removes the
  hidden `.xtrace-keeping-…` copies in `~/.agents/skills`; a crash right
  after the name was claimed can leave an empty `~/.agents/skills/<name>`
  folder, which blocks that name until you remove it.
  The skill then shows in Ready to add (it has a valid rubric) or Your skills
  (Make a rubric), like any installed skill.
- **Not yet.** Updating a kept skill to a newer commit, removing it from the
  Mac, private repositories, other sites, and choosing another name on a
  clash.

## Making a rubric

The Add a skill dialog starts with three steps (pick a skill, make a rubric,
add it) and lists the installed skills in four groups, decided in Rust
(`crate::grading_picker`, each row's `group`): **Ready to add**, **Your
skills** (every skill a rubric can be made for), and two groups folded at
the bottom, **Can't be scored** (skills Claude refused, below) and **Already
on Scores** (built in, added, or another skill on Scores has the id). The
search filters every group; a folded group shows how many of its skills
match. In Your skills, a skill without a rubric gets a
**Make a rubric** button; one whose rubric is refused (its reason is shown)
gets **Remake rubric**. Neither is offered for a skill that Scores
already has under another skill: one marked "On Scores (built in)", or one
whose name is the id of a skill added from another folder. A skill that is
itself on Scores can have its broken rubric remade; its card keeps its id.
Pressing it runs Claude Code in the
background with XTrace's own helper skill, `xtrace-rubric-maker`
(`crates/xt-judge/rubric-maker/SKILL.md`, bundled with the app and never
installed into your skill folders). It uses about a minute of your Claude
usage; the run's limit is 10 minutes.

- **What Claude sees.** A copy of the skill in a private staging folder, its
  only working folder, read-only. The helper tells it to read the skill (its
  `SKILL.md`, scripts, templates and documents; nothing is run), work out what
  it checks and what its output looks like, and write one file,
  `OUT/xtrace-rubric.md`, in the format above. `OUT` is the only folder it can
  write to. The run is contained exactly like a grading (below): the same
  Claude Code command line, sandbox, settings, version check, deadline, Cancel
  and quit handling. A rubric job and a grading never run together. For
  Remake, the copy leaves out the skill's own `xtrace-rubric.md` entry
  (whatever it is, even a link out of the folder or to nothing); the refused
  rubric that applies (the folder's or the one XTrace keeps) is placed there
  as a read-only copy when it is a regular file, and the prompt says why it
  was refused and whether the copy is there.
- **How XTrace uses the rubric, as the helper tells Claude.** The skill will
  run headless on a project: the project is read-only, there is no network,
  no MCP servers or plugins are loaded, and nobody answers questions. What
  the skill writes lands in a grade folder, and Claude then scores only that
  output with the rubric. So the helper asks for criteria built only from
  checks the skill itself performs and reports (never new ones), each saying
  what to award when its output is missing, partial, or the skill stopped
  because it needed something it could not get (usually 0).
- **Refusal.** When Claude judges the skill cannot be used to score projects
  (it builds, writes, publishes or looks things up instead of reviewing a
  project, or it cannot work headless, offline and read-only), the helper
  writes `OUT/xtrace-refusal.md` instead: one short plain reason of at most
  300 characters. XTrace reads it as text only (a regular file of at most
  4 KB, UTF-8 without control characters; spaces are tidied and a longer
  reason is cut at 300 characters), saves nothing and does not try again.
  The job ends `refused`, and the skill moves to **Can't be scored** with the
  reason and **Try again**. The refusal is kept across restarts (below).
- **Checked by XTrace.** The file is read with the same rubric reader. If it
  is missing or refused, or the refusal cannot be read, or both files were
  written, Claude gets one more try whose prompt names that reason (and, for
  Remake, still why the old rubric was refused); if that fails too, the job
  fails with that reason and nothing is saved. Anything else the run wrote is
  ignored.
- **Its version.** XTrace sets the saved rubric's header `version`: `1` for
  a first rubric; for Remake, the old rubric's `version` plus one when it is
  a plain whole number, otherwise `2`. Grades made with the new rubric show
  the new version.
- **Its id.** When another skill already has the rubric's `id` (a built-in
  skill, a skill on Scores, or another installed skill whose rubric is
  ready), XTrace appends the smallest free `-2` to `-99` and checks the file
  again; with none free, the job fails. Remaking the rubric of a skill that is
  on Scores keeps the id its card has.
- **Only XTrace saves it**, after the check:
  - A skill in `~/.claude/skills`, `~/.agents/skills` or `~/.codex/skills`
    gets `xtrace-rubric.md` next to its `SKILL.md`, in the skill's real folder
    (links on the way are resolved). The file is written to a hidden
    temporary file and linked into place, so an existing rubric is never
    overwritten: one that appeared during the job fails it instead. Remaking
    first moves the old file to `xtrace-rubric.md.bak` (over an older
    backup); if the new file then cannot be put in place, the old one is
    moved back before the job fails.
  - A Claude Code plugin's skill, and a skill folder XTrace cannot write, gets
    its rubric in XTrace's rubric store (below). A folder that cannot be
    written and has its own refused rubric cannot be remade: its own file
    would still win.
  - A job cancelled before its rubric is saved saves nothing; XTrace checks
    for a cancel once more right before saving.
  - Known limit: a plugin's skill whose own folder ships a broken
    `xtrace-rubric.md` is remade in that plugin folder (the folder's own file
    is the one that applies), so a plugin update may bring the plugin's
    broken version back.
- The skill then shows as **Ready to add**, with its criteria, and can be added
  like any other. A job's outcome is shown on its row only while it still
  matches the skill's rubric: a made rubric while the skill is ready; a
  failure, refusal or cancel while it is not. Outcomes are kept in memory
  only, until XTrace quits, except a refusal.
- **Kept refusals.** XTrace keeps the latest refusal of each skill in
  `<XTrace data folder>/rubric-refusals.json` (`xt_judge::refusals`): the
  skill's location, the job ID, Claude's short reason, when, and a
  fingerprint of the skill taken when the job started (a SHA-256 of its
  `SKILL.md` and of its folder listing: each entry's path, kind and file
  size, links listed but not followed, its own `xtrace-rubric.md` and backup
  left out). One rule (`xt_judge::refusals::in_force`) decides whether a
  kept refusal applies: only while the skill's fingerprint now is the same.
  A skill that was edited or updated since (another `SKILL.md`, a file added,
  removed or of another size) is back in Your skills; an edit that keeps
  every other file's size is noticed only in `SKILL.md`. **Try again** drops
  the kept refusal as its job starts, whatever that job then does. A kept
  reason is shown only where Try again is (Can't be scored); a skill Scores
  already has shows none. The file holds at most 200 refusals (the oldest go
  first), each with a location of at most 1,024 bytes, a reason of at most
  300 characters, no control characters, so a full file stays well under the
  2 MB XTrace reads; a refusal outside those bounds, or one whose skill could
  not be fingerprinted when its job started, is not kept (the skill stays in
  Your skills). A file that cannot be read or is not this layout counts as
  none, and one that cannot be written keeps the change until XTrace quits. A
  reason is short text Claude wrote about the skill, never conversation
  content. **Delete stored content** removes the file and the refusals in
  memory (the rubrics XTrace keeps stay), so refused skills are back in Your
  skills.

## Where rubrics are kept

**One rule** decides which rubric applies to a skill: the skill folder's own
`xtrace-rubric.md` whenever the folder has one (valid or not), otherwise the
one in XTrace's rubric store, otherwise none. The store is
`<XTrace data folder>/rubrics/<key>/xtrace-rubric.md`, where `<key>` is a safe
folder name made from the skill's location: a plugin's skill by its plugin and
name (`plugin:<name>@<marketplace>/<skill>`), so a plugin update that moves the
plugin to a new version folder keeps the rubric; any other skill by its
folder path. Discovery, adding a skill and every grading use this rule; a run
of a skill whose rubric is in the store gets a copy of it in its staged skill
folder.

## What a run writes

`result.json` follows XTrace's grader protocol v1 (`crates/xt-judge/src/protocol.rs`):
`score` (0–100), `summary`, `insights` (the findings), `report` (an HTML
report the skill wrote, or `null`), `complete`, and `criteria`: one
`{name, max, awarded}` entry per rubric criterion, in the rubric's order. XTrace
checks that the criteria are exactly the rubric's and that the awarded points
add up to the score; anything else fails the run as an invalid result. The
criteria are checked but not stored: XTrace stores and shows the score,
summary, findings and report.

## How a run is contained

The run uses the user's Claude Code, but none of their personal Claude Code
settings: no user, project or local settings file is loaded, so their own
permission rules, sandbox exceptions, hooks and plugins do not apply. It runs
in "don't ask" mode inside Claude Code's sandbox: the project folder is
read-only, only the run's grade folder is writable, and the network is always
off. Well-known places that hold secrets (SSH and GPG keys, cloud, database and
package-registry credentials, other coding tools' sign-ins and settings,
keychain files and browser profiles; the list is `SECRET_LOCATIONS` in `crates/xt-judge/src/runner.rs`)
cannot be read: the sandbox refuses them to every command, and `Read` deny
rules refuse them to Claude Code's own file tools. What the bundled
claude-md-doctor reads, such as `~/.claude/projects`, stays readable. On
macOS the sandbox also hides whether anything in those places exists, so a
project instruction that names one (say `~/.aws/config`) is reported as a
missing file. If the sandbox is not available, Claude Code refuses to start and the run
fails with "sandbox unavailable". Claude Code older than 2.1.296 (the version
these settings were tested with), or one whose version cannot be read, is
refused before the run starts. MCP servers are off and the run's transcript is
not saved.

The built-in claude-md-doctor rubric is
`vendor/claude-md-doctor/skill/xtrace-rubric.md`.
