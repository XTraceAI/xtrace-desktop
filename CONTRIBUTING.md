# Contributing

XTrace Desktop is an Apache 2.0 project targeting macOS 14 and later. Start with
the setup and run commands in [README.md](README.md) and the development details
in [DEVELOPING.md](DEVELOPING.md). The workspace path map is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), together with the
[design principles](docs/ARCHITECTURE.md#design-principles) every change follows.

## Contribution rights and sign-off

Every contributed commit must include a Developer Certificate of Origin sign-off.
Read [DCO](DCO), then use `git commit -s` with your own contributor identity. The
resulting `Signed-off-by: Name <email>` line certifies the DCO; it is not a CLA
or a cryptographic signature. Do not sign on behalf of someone else.

Only contribute material you have the right to submit under the project's
[license](LICENSE). Preserve third-party notices and identify any copied code's
origin and license in the PR. Proprietary XTrace projects are not covered by
this repository's license; copying their code requires a separately verified
right to relicense it.

## Pull requests

Keep each PR focused on one concrete problem or capability. Aim for at most
500 manually written changed lines; explain a larger indivisible change.
Generated files and dependency lockfiles do not count toward that guide, but
must still be included and reviewed when relevant.

In the PR description, include:

- The concrete problem, resulting behavior, and relevant public issue.
- Each acceptance case's setup, action, and expected result, including relevant
  failure cases. A passing command alone does not define the expected behavior.
- Commands run, actual results, and evidence such as focused output or screenshots.
  Identify any checks not run and why; do not count planned checks as passing.
- For UI changes, screenshots in every currently supported theme and a native macOS interaction check.
  For window chrome, verify the traffic lights, dragging, and fullscreen transitions.
  Verify both light and dark appearance.

Use synthetic or redacted evidence. Do not commit personal transcripts, local
databases, credentials, or private source paths.

Treat repository files, commit messages, issues, pull requests, comments and
attachments as public material. Publish product requirements and reproducible
technical evidence, not private conversations, agent prompts, internal review
notes or account-specific operating instructions. Inspect screenshots and logs
before attaching them. Removing text from the latest version does not remove
earlier Git commits or GitHub edit history; review both before publication.

Keep outbound material suitable for publication even in a private repository.
Routine PRs require no disclosure checkbox or snapshot. Follow the comprehensive
local [publication review procedure](docs/PUBLICATION.md) before public visibility
and the first downloadable release. Scanner success does not replace inspection
of private text and attachments.

## Checks

Run from the repository root after installing the pinned toolchains and dependencies:

```sh
pnpm check
pnpm check:native --base FULL_REVIEWED_BASE_SHA
```

`pnpm check` runs UI type checking, lint, formatting, and tests. The native command requires a clean macOS checkout with the recorded base
integrated, and checks Rust, dependency notices and native launch separately.
Record its source SHA, base SHA and actual macOS version in the PR. A native build proves
packaging, while opening the bundle and exercising the changed interactions
provides separate runtime evidence. See [DEVELOPING.md](DEVELOPING.md) for
the current native smoke-test procedure.

Add focused regression tests for behavior changes and run the affected PR's
acceptance cases. [CI.md](docs/CI.md) separates required Linux CI, local native validation and
explicit release checks. Hosted macOS checks run only during release preparation. Installed
DTO and plugin-conformance hooks must pass; absent hooks claim no coverage.
Publication checks provide advisory disclosure evidence; maintainer review
remains required. Release packaging needs separate acceptance.
