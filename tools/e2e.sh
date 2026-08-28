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
# What the application ships with, taken from the catalogue itself so the two
# cannot drift apart. Without this the run took whichever weights sorted first,
# which is not a choice anybody made: a suite that scores the small model and
# says nothing about which one it scored is a suite that reads as a regression
# every time somebody puts another file in the folder.
preferred="$(grep -o '"[A-Za-z0-9._-]*\.gguf"' src/settings.rs | tr -d '"' | head -1)"
if [ -z "$want" ] && [ -n "$preferred" ] && [ -e "$MODELS/$preferred" ]; then
    model="$preferred"
fi
if [ -z "$model" ]; then
    for f in "$MODELS"/*.gguf; do
        [ -e "$f" ] || continue
        base="$(basename "$f")"
        if [ -z "$want" ] || [[ "$base" == *"$want"* ]]; then model="$base"; break; fi
    done
fi
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
