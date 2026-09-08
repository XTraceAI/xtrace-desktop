# Contributing

XTrace Desktop is an Apache 2.0 project targeting macOS 14 and later. Start with
the setup and run commands in [README.md](README.md) and the development details
in [DEVELOPING.md](DEVELOPING.md). The workspace path map is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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

Keep each PR focused on one scheduled card or its named child. Aim for at most
500 manually written changed lines; explain a larger indivisible change.
Generated files and dependency lockfiles do not count toward that guide, but
must still be included and reviewed when relevant.

In the PR description, include:

- The concrete problem, resulting behavior, and scheduled card/specification IDs.
- Each acceptance case's setup, action, and expected result, including relevant
  failure cases. A passing command alone does not define the expected behavior.
- Commands run, actual results, and evidence such as focused output or screenshots.
  Identify any checks not run and why; do not count planned checks as passing.
- For UI changes, screenshots in every currently supported theme and a native macOS interaction check.
  For window chrome, verify the traffic lights, dragging, and fullscreen transitions.
  The scaffold supports dark appearance; FND-05 introduces the shared light/dark theme system.

Use synthetic or redacted evidence. Do not commit personal transcripts, local
databases, credentials, or private source paths.

Treat repository files, commit messages, issues, pull requests, comments and
attachments as public material. Publish product requirements and reproducible
technical evidence, not private conversations, agent prompts, internal review
notes or account-specific operating instructions. Inspect screenshots and logs
before attaching them. Removing text from the latest version does not remove
earlier Git commits or GitHub edit history; review both before publication.

Apply the [publication review procedure](docs/PUBLICATION.md) even in a private
repository. Run `pnpm publication:test` and `pnpm security:scan`, and check the PR
template's disclosure attestation after reviewing the final text, linked issues,
comments and attachments. Scanner success does not replace this semantic review.

## Checks

Run from the repository root after installing the pinned toolchains and dependencies:

```sh
pnpm check
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm tauri build --debug --bundles app
```

`pnpm check` runs UI type checking, lint, formatting, and tests. The Rust commands
check the full workspace separately. A native build proves
packaging, while opening the bundle and exercising the changed interactions
provides separate runtime evidence. See [DEVELOPING.md](DEVELOPING.md) for
the current native smoke-test procedure.

Add focused regression tests for behavior changes and run the affected PR's
acceptance cases. Fixture loading, DTO parity, browser coverage, and release
checks join this process as their owning foundation cards land. Publication
checks provide a focused disclosure gate; broader build/test CI, automated DCO
checks and dependency-license inventory remain future work.
