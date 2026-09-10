#!/bin/sh
set -eu

# This local/native gate must execute the real producer, never report a skip.
: "${AGENT_PLUGINS_DIR:?Set AGENT_PLUGINS_DIR to the pinned plugin root}"
PYTHON="${PYTHON:-python3}"
export PYTHON
"$PYTHON" --version >/dev/null
cargo test -p xt-server --test conformance --locked -- --nocapture
