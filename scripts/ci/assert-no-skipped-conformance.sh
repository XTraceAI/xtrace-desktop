#!/bin/sh
# Validate captured `cargo test -- conformance --nocapture` output: every
# inventoried test must have run and passed, and nothing may have skipped
# itself, been ignored or reported zero tests. Usage: <log file> [inventory].
set -eu
LOG="${1:?Usage: assert-no-skipped-conformance.sh <cargo test log> [inventory]}"
INVENTORY="${2:-$(dirname "$0")/conformance-inventory.txt}"
test -s "$LOG" || { echo "Conformance log is missing or empty: $LOG" >&2; exit 1; }
test -s "$INVENTORY" || { echo "Conformance inventory is missing or empty: $INVENTORY" >&2; exit 1; }
status=0
skipped="$(grep -E '^SKIP [A-Za-z0-9_:]+:' "$LOG" | sed -E 's/^SKIP ([A-Za-z0-9_:]+):.*/\1/' | sort -u)"
if grep -q "SKIP " "$LOG"; then
  echo "Conformance tests skipped themselves; a skip is not evidence:" >&2
  grep "SKIP " "$LOG" >&2
  status=1
fi
if grep -Eq '^test [A-Za-z0-9_:]+ \.\.\. ignored' "$LOG"; then
  echo "Conformance tests were ignored:" >&2
  grep -E '^test [A-Za-z0-9_:]+ \.\.\. ignored' "$LOG" >&2
  status=1
fi
if grep -Eq '^test [A-Za-z0-9_:]+ \.\.\. FAILED' "$LOG" || grep -Eq '^test result: FAILED' "$LOG"; then
  echo "Conformance tests failed." >&2
  status=1
fi
count=0
while IFS= read -r name; do
  [ -n "$name" ] || continue
  if printf '%s\n' "$skipped" | grep -Fxq "$name"; then
    echo "Required conformance test skipped itself instead of running: ${name}" >&2
    status=1
  elif grep -Eq "^test ${name} \.\.\. ok\$" "$LOG"; then
    count=$((count + 1))
    echo "executed ${name}"
  else
    echo "Required conformance test did not run and pass: ${name}" >&2
    status=1
  fi
done < "$INVENTORY"
expected=$(grep -c . "$INVENTORY")
echo "executed ${count} of ${expected} required conformance tests"
[ "$count" -gt 0 ] || { echo "Zero conformance tests ran." >&2; status=1; }
exit "$status"
