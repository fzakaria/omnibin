#!/usr/bin/env bash
# Mounts omnibin over a small crawl and checks the one property the whole
# project rests on: looking at a package is free and reading one is not.
#
# Crawls a few hundred listings rather than using a published artifact, so the
# test exercises the crawler, the index builder and the filesystem together
# and does not need a data release to exist.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'fusermount3 -u "$WORK/store" 2>/dev/null || true;
      fusermount3 -u "$WORK/tree" 2>/dev/null || true;
      rm -rf "$WORK"' EXIT

MULTIVERSE="${MULTIVERSE_ROOT:-$WORK/multiverse}"
if [ ! -d "$MULTIVERSE" ]; then
  echo "== fetching the multiverse artifacts the index is built from"
  git clone --depth 1 https://github.com/fzakaria/nixpkgs-multiverse "$MULTIVERSE"
  "$MULTIVERSE/tools/fetch-data.sh" 2>/dev/null || true
fi

DATA="$MULTIVERSE/index/.outpaths/data"
if [ ! -f "$DATA/outpaths-x86_64-linux.json" ]; then
  echo "mount-test: no multiverse artifacts in $DATA; skipping" >&2
  exit 0
fi

mkdir -p "$WORK"/{store,tree,cache}

echo "== crawling a sample of listings"
"$ROOT/tools/crawl-listings.py" \
  --outpaths "$DATA/outpaths-x86_64-linux.json" \
  --out "$WORK/listings.jsonl.zst" --limit 400 --jobs 32

echo "== building the index"
"$ROOT/tools/build-index.py" \
  --listings "$WORK/listings.jsonl.zst" \
  --outpaths "$DATA/outpaths-x86_64-linux.json" \
  --info-indexed "$DATA/info-indexed.json.gz" \
  --versions "$MULTIVERSE/index/versions.json" \
  --revisions "$MULTIVERSE/revisions.json" \
  --system x86_64-linux --out "$WORK/omnibin.db"

echo "== mounting"
OMNIBIN_DB="$WORK/omnibin.db" cargo run --release --quiet -- mount \
  --store "$WORK/store" --tree "$WORK/tree" --cache-dir "$WORK/cache" &
MOUNT_PID=$!

for _ in $(seq 1 50); do
  [ -e "$WORK/tree/README.md" ] && break
  sleep 0.2
done

# The tree lists bare names and resolves versioned ones.
test "$(ls "$WORK/tree/bin" | wc -l)" -gt 0
NAME="$(ls "$WORK/tree/bin" | head -1)"
test -L "$WORK/tree/bin/$NAME"

# Looking at a package reads its listing and downloads nothing.
BASE="$(readlink "$WORK/tree/bin/$NAME" | cut -d/ -f4)"
ls "$WORK/store/$BASE/bin" > /dev/null
downloads_after_looking="$(ls "$WORK/cache/store" 2>/dev/null | wc -l)"
if [ "$downloads_after_looking" -ne 0 ]; then
  echo "mount-test: listing a package fetched $downloads_after_looking path(s)" >&2
  exit 1
fi

# Reading one byte out of it fetches exactly one path.
head -c 1 "$WORK/tree/bin/$NAME" > /dev/null
downloads_after_reading="$(ls "$WORK/cache/store" | wc -l)"
if [ "$downloads_after_reading" -ne 1 ]; then
  echo "mount-test: reading a file fetched $downloads_after_reading path(s)" >&2
  exit 1
fi

kill "$MOUNT_PID" 2>/dev/null || true
echo "ok: looking is free, reading costs one path"
