#!/usr/bin/env bash
# Drive the whole application against the real model and check what it did.
#
# Everything happens in a vault made for the run, with its own settings file,
# so it cannot touch the notes you keep. Weights are the ones already in
# ./models - nothing is downloaded.
#
#   tools/e2e.sh                      the lot, on the first model it finds
#   tools/e2e.sh Ornith               on the first model whose name matches
#   E2E_ONLY="turned down" tools/e2e.sh    one scene
#
# Exits non-zero if any scene failed.
set -euo pipefail
cd "$(dirname "$0")/.."

MODELS="${PIXUI_MODELS:-models}"
want="${1:-}"
model=""
for f in "$MODELS"/*.gguf; do
    [ -e "$f" ] || continue
    base="$(basename "$f")"
    if [ -z "$want" ] || [[ "$base" == *"$want"* ]]; then model="$base"; break; fi
done
if [ -z "$model" ]; then
    echo "no weights in $MODELS${want:+ matching \"$want\"} - the assistant would be the stub" >&2
    echo "put a .gguf there, or run the app and fetch one from settings" >&2
    exit 2
fi

sandbox="$(mktemp -d "${TMPDIR:-/tmp}/notes-e2e.XXXXXX")"
trap 'rm -rf "$sandbox"' EXIT
mkdir -p "$sandbox/vault"
# Its own settings, so the run cannot pick up - or disturb - yours. Looking
# things up stays off: this checks the application, not somebody's network.
cat > "$sandbox/settings.conf" <<CONF
assist = on
web = off
model = $model
CONF

echo "sandbox $sandbox"
cargo build --release --quiet
PIXUI_NOTES_DIR="$sandbox/vault" \
PIXUI_CONFIG="$sandbox/settings.conf" \
PIXUI_MODELS="$(cd "$MODELS" && pwd)" \
    ./target/release/notes --e2e
