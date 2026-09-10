#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
committed="${1:-$repo_root/apps/desktop/ui/src/data/generated}"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
cd "$repo_root"
python3 - "$committed" "$repo_root" <<'PYGUARD'
from pathlib import Path
import sys
committed, repo = map(Path, sys.argv[1:])
if committed.is_symlink():
    sys.exit('generated DTO root is a symlink')
default = repo / 'apps/desktop/ui/src/data/generated'
if committed.absolute() == default:
    for path in (committed, *committed.parents):
        if path == repo:
            break
        if path.is_symlink():
            sys.exit('generated DTO parent is a symlink')
    if not committed.resolve().is_relative_to(repo.resolve()):
        sys.exit('generated DTO directory escapes the repository')
PYGUARD
cargo xtask dto-export --out "$temporary/generated"
python3 - "$temporary/generated" "$committed" <<'PY'
from pathlib import Path
import sys
expected, committed = map(Path, sys.argv[1:])
def contents(root):
    if root.is_symlink():
        raise ValueError('generated DTO root is a symlink')
    if not root.is_dir():
        raise ValueError('generated DTO directory is missing')
    result = {}
    for path in root.rglob('*'):
        if path.is_symlink():
            raise ValueError('generated DTO directory contains a symlink')
        if path.is_file():
            result[path.relative_to(root).as_posix()] = path.read_bytes()
    return result
try:
    if contents(expected) != contents(committed):
        raise ValueError('generated DTO files are missing, stale or unexpected; run cargo xtask dto-export')
except ValueError as error:
    sys.exit(str(error))
print('Generated DTO parity passed.')
PY

if [[ $# == 0 ]]; then
  node --test scripts/ci/native/dto.test.mjs
fi
