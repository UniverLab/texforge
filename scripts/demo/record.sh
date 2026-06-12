#!/usr/bin/env bash
# Records the demo scenario with asciinema and renders assets/texforge.gif.
#
# Requirements: asciinema (v2), agg, and texforge on PATH.
# Usage: ./scripts/demo/record.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CAST="$(mktemp --suffix=.cast)"
GIF="$REPO_ROOT/assets/texforge.gif"

# Prefer the freshly built binary over any installed release.
if [ -x "$REPO_ROOT/target/release/texforge" ]; then
    export PATH="$REPO_ROOT/target/release:$PATH"
fi

COLUMNS=100 LINES=30 asciinema rec --overwrite \
    -c "bash $REPO_ROOT/scripts/demo/scenario.sh" "$CAST"

agg --font-size 16 "$CAST" "$GIF"
rm -f "$CAST"

echo "Demo written to $GIF"
