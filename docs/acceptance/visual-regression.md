# Visual regression acceptance

| Check                                                  | Expected result                                                                                                                                                                                                |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm check`                                           | Types, lint, formatting and unit tests pass. Contract tests reject missing mappings/images, duplicate themes, wrong dimensions, changed image bytes, stale inventory/fonts/runner metadata and missing review. |
| `pnpm parity:baselines`                                | Capture all 40 candidate PNGs from a clean source commit, then pass a separate repeat comparison without changing candidate bytes. Approved baselines remain untouched.                                        |
| `pnpm parity:review /path/to/candidates`               | All 40 comparisons pass on the manifest's macOS/WebKit environment. This verifies reproducibility, not visual approval.                                                                                        |
| `pnpm parity:probe /path/to/candidates`                | A deliberate border change fails exactly one screenshot comparison and produces expected/actual/diff evidence. Baseline bytes remain unchanged.                                                                |
| `pnpm parity:accept /path/to/candidates REVIEW_DIGEST` | Only a reviewed candidate digest with current rendering source and metadata can be copied into the baseline directory. Record the visual review separately.                                                    |
| `pnpm parity`                                          | All 40 approved comparisons pass with updates disabled. Missing/stale expectations fail rather than being silently created.                                                                                    |

Coverage is eight Sidebar, twelve TopBar and twenty StatTile images across both
themes. The map fixes CSS dimensions at 228×900, 1200×44 and 300×52 respectively.
PNG dimensions are twice those sizes. The normal gallery smoke, complete local
gallery sweep and production-exclusion checks retain their existing contracts.

The metric geometry browser check also exercises a long label with a measured
value, unit and delta at 300px. The label may truncate; ordinary metric values
must remain fully visible in both themes and browser engines. Gallery deltas
use ratios so the illustrative changes display as 4% and -8%.

The PR records the exact source and candidate review identity, actual macOS and
WebKit versions, reproducibility results and deliberate-regression evidence.
Initial candidate generation alone does not satisfy acceptance: the visual review
and successful comparison against approved baselines are required before merge.

See [the local workflow](../VISUAL_TESTS.md). Routine hosted CI runs only the
small contract tests as part of `pnpm check`; these macOS screenshot comparisons
remain local and add no hosted browser or macOS step.
