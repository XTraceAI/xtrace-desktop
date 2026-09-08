#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
cd "$repo_root"
# Only F1 is populated. Skeletons cannot produce meaningful shell counts.
cargo xtask fixture-export F1 --shell --out "$temporary/F1.json"
mkdir -p apps/desktop/ui/fixtures
cp "$temporary/F1.json" apps/desktop/ui/fixtures/F1.json
