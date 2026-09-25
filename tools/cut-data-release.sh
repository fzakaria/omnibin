#!/usr/bin/env bash
# Cuts (or tops up) a dated data release: uploads every artifact whose bytes
# differ from what data-pins.json already pins, then repoints the pins.
#
# Cuts are delta-sized by construction. A published listings shard hashes
# identically to its pin forever — the listing for a digest describes bytes
# that cannot change — so a normal cut uploads the current generation's shards
# and the rebuilt database, and nothing else.
#
# Needs `gh` authenticated with repo scope, and the artifacts already built by
# tools/crawl-listings.py, tools/shard-listings.py and tools/build-index.py.
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

if [ ! -d "$DATA/listings" ]; then
  echo "cut-data-release: no $DATA/listings; run shard-listings.py first" >&2
  exit 1
fi

# The cut's candidate set: every published listings shard, and the database
# built from all of them.
CANDIDATES=()
while IFS= read -r f; do
  CANDIDATES+=("$f")
done < <(find "$DATA/listings" -name 'listings-*.jsonl.zst' | sort)
for db in "$DATA"/omnibin-*.db; do
  [ -f "$db" ] && CANDIDATES+=("$db")
done

if [ ${#CANDIDATES[@]} -eq 0 ]; then
  echo "cut-data-release: nothing to upload" >&2
  exit 1
fi

# Only bytes that moved get uploaded: compare each candidate's narHash to the
# pinned one.
CHANGED=()
for f in "${CANDIDATES[@]}"; do
  name=$(basename "$f")
  current=$(nix hash path --sri --type sha256 "$f")
  pinned=$(python3 -c "
import json, os, sys
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

# Create the tag on first use. Assets on a dated tag are immutable by
# convention: never overwritten, never deleted.
if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release create "$TAG" --repo "$REPO" \
    --title "omnibin data $TAG" \
    --notes "File listings crawled from cache.nixos.org, and the index built from them. Addressed by data-pins.json."
fi

gh release upload "$TAG" --repo "$REPO" "${CHANGED[@]}"

"$HERE/bump-data-pin.sh" "$TAG" "${CHANGED[@]}"
