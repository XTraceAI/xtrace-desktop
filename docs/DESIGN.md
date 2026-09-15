# UI conventions

Reuse the existing [component kit](../apps/desktop/ui/src/kit/index.ts) and
[gallery](GALLERY.md) before adding a new primitive. Keep renderer data access behind
[DataSource](../apps/desktop/ui/src/data/DataSource.ts); product metrics belong in
Rust rather than duplicated component calculations.

Use the existing theme tokens and semantic CSS variables. `pnpm lint` checks raw
hex usage; `pnpm check` includes component tests. Preserve keyboard navigation,
focus indicators, accessible names, and independent popup behavior in both themes.
Unmeasured values display `—`, never a fabricated zero. Explain metric definitions
through the existing definition components where appropriate.

Gallery samples demonstrate components; snapshots do not independently approve a
product design. Follow [visual verification](VISUAL_TESTS.md), compare against the
approved design reference, and review changed states in both light and dark themes.
Use synthetic data in screenshots. Do not copy private design conversations into
repository files or PRs.
