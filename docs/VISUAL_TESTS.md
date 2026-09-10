# Local visual regression checks

The gallery is the input to 40 WebKit screenshot comparisons: four Sidebar
states, six TopBar states and ten StatTile states, each in dark and light themes.
These tests detect changes to reviewed component appearance. They do not certify
completed product screens, native window behavior or independent design parity.

Run these checks locally on macOS. Routine GitHub Actions keeps its existing
gallery smoke suite; no hosted macOS job is added. Playwright supplies screenshot
stabilization, PNG comparison and before/after/diff artifacts through its
[screenshot assertions](https://playwright.dev/docs/test-snapshots).

## Review the initial baselines or an intentional change

Install the pinned tools and browser, then commit the source you intend to capture:

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm --dir apps/desktop/ui exec playwright install webkit
pnpm parity:baselines
```

The command requires a clean checkout. It writes a new candidate directory under
ignored `artifacts/parity/`, then independently compares another capture against
those candidates. It never writes approved images. Open the printed directory's
`index.html` to review all 40 images in paired themes at their CSS dimensions;
click an image for its full 2× resolution.

The manifest records the exact source commit, comparison map, gallery inventory,
local font hashes, macOS version/architecture, Playwright/WebKit versions and
revision, viewport, DPR, locale, timezone and comparison thresholds. Reviewers
must inspect component layout, text, icons, colors and intentional differences
from the design reference. Generating candidates does not approve their pixels.

When approved baselines already exist, capture preserves their images under
`previous/` and emits comparison results under `changes/`. Inspect each changed
image's expected/actual/diff PNGs there; a new mapping has no prior image. Runner
or font upgrades require new candidate review. This update comparison supports
review, not acceptance against the old environment.

After reviewing the exact candidate set, explicitly promote its printed digest:

```sh
pnpm parity:review /path/to/candidates
pnpm parity:probe /path/to/candidates
pnpm parity:accept /path/to/candidates REVIEW_DIGEST
pnpm parity
```

Acceptance checks the digest, image bytes and dimensions, current map/fonts/runner
and unchanged captured rendering source. It preserves previous baselines in
ignored output, copies only the reviewed PNGs and manifest into
`apps/desktop/ui/e2e/parity/baselines`, and leaves the changes for normal Git
review. Record who reviewed the images and why any visual changes are intended in
the PR. This local record is a review aid, not an access-control mechanism.

## Normal validation

`pnpm parity` reads approved baselines with snapshot updates disabled. Missing
images, unapproved/stale manifests, removed map entries and mismatched font or
runner identities fail. Changed rendering is compared at `maxDiffPixelRatio=0.002`
and `threshold=0.1`. Fonts must load locally; browser errors and external HTTP
requests fail. Neither a comparison failure nor a missing image regenerates a
baseline. Review the ignored output and fix the cause.

`pnpm parity:probe` applies a visible 12px border change to one disposable sidebar
document. It succeeds only when exactly that comparison fails, before/after/diff
images exist, and baseline PNG hashes remain unchanged. It changes no source or
stored app data.

The suite owns port 5184 and runs one headless WebKit worker at 2880×1120, DPR 2,
UTC and `en-US`. Run it sequentially with other browser suites. Screenshots are
cropped to the gallery frame, including its background, at the declared size.
Pixel results must be reproduced on the recorded macOS environment; a newer OS
does not demonstrate the application's macOS 14 support floor.
