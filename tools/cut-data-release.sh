#!/usr/bin/env bash
# Cuts (or tops up) a dated data release: uploads every artifact whose bytes
# differ from what data-pins.json already pins, then repoints the pins.
#
# Cuts are delta sized by construction. A published listings shard hashes
# identically to its pin forever, because the listing for a digest describes
# bytes that cannot change, so a normal cut uploads the rebuilt database, the
# narinfos, and whatever listings the last crawl added.
#
# Safe to run twice. The second run sees the pins it just wrote and uploads
# only what is still missing, which is how a big first cut can be done in two
# passes: the database on its own, then the listings behind it.
#
# Needs `gh` authenticated with repo scope, and the artifacts already built by
# the tools in docs/building-the-index.md.
#
# Usage:
#   tools/cut-data-release.sh              # tag data-<today, UTC>
#   tools/cut-data-release.sh data-20260924
set -euo pipefail

ROOT="${OMNIBIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HERE="$(cd "$(dirname "$0")" && pwd)"
DATA="$ROOT/data"
TAG="${1:-data-$(date -u +%Y%m%d)}"
REPO="fzakaria/omnibin"
# Every system that has been built, rather than one named here. A system is
# published when its database exists, and the flake serves whichever systems
# the pins name.
DBS=()
for f in "$DATA"/omnibin-*.db; do
  [ -f "$f" ] && DBS+=("$f")
done

if [ ${#DBS[@]} -eq 0 ]; then
  echo "cut-data-release: no $DATA/omnibin-*.db; run tools/build-index.py first" >&2
  exit 1
fi

# The databases first, because they are what the flake needs and what a
# consumer cannot reconstruct without a full crawl. Then the raw crawl behind
# them.
CANDIDATES=("${DBS[@]}")
for f in "$DATA"/narinfos*.jsonl.zst "$DATA"/listings/listings-*.jsonl.zst; do
  [ -f "$f" ] && CANDIDATES+=("$f")
done

# Only bytes that moved get uploaded: compare each candidate's narHash to the
# pinned one.
CHANGED=()
for f in "${CANDIDATES[@]}"; do
  name=$(basename "$f")
  current=$(nix hash path --sri --type sha256 "$f")
  pinned=$(python3 -c "
import json, os
p = '$ROOT/data-pins.json'
pins = json.load(open(p)) if os.path.exists(p) else {'files': {}}
print(pins['files'].get('$name', {}).get('narHash', ''))
")
  if [ "$current" != "$pinned" ]; then
    CHANGED+=("$f")
  fi
done

if [ ${#CHANGED[@]} -eq 0 ]; then
  echo "cut-data-release: every artifact already pinned at its current bytes"
  exit 0
fi

echo "cut-data-release: $TAG, uploading ${#CHANGED[@]} of ${#CANDIDATES[@]} artifacts"

# The notes are generated from the database that is being published, so the
# numbers on the release and the numbers in the README come from one place.
NOTES=$(mktemp)
trap 'rm -f "$NOTES"' EXIT
python3 "$HERE/status.py" --db "${DBS[@]}" --notes --tag "$TAG" > "$NOTES"

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release edit "$TAG" --repo "$REPO" --notes-file "$NOTES"
else
  gh release create "$TAG" --repo "$REPO" \
    --title "omnibin data $TAG" --notes-file "$NOTES"
fi

# Assets on a dated tag are immutable by convention: never overwritten, never
# deleted. --clobber is deliberately not passed, so a re-upload of changed
# bytes under a name that already exists fails loudly.
gh release upload "$TAG" --repo "$REPO" "${CHANGED[@]}"

"$HERE/bump-data-pin.sh" "$TAG" "${CHANGED[@]}"
