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
# Every mountpoint this script makes, unmounted before the directory goes.
# A function rather than a quoted loop, so the shell and shellcheck can both
# read it.
cleanup() {
  local point
  for point in store tree store2 tree2; do
    fusermount3 -u "$WORK/$point" 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

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

# Built once, up front. `cargo run` compiles on its first call, which on a
# dirty tree takes longer than the readiness loop below waits, so the mount
# looked like it had failed when it had not yet started.
echo "== building omnibin"
cargo build --release --quiet
OMNIBIN="$ROOT/target/release/omnibin"

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
OMNIBIN_DB="$WORK/omnibin.db" "$OMNIBIN" mount \
  --store "$WORK/store" --tree "$WORK/tree" --cache-dir "$WORK/cache" &
MOUNT_PID=$!

for _ in $(seq 1 50); do
  [ -e "$WORK/tree/README.md" ] && break
  sleep 0.2
done

if [ ! -e "$WORK/tree/README.md" ]; then
  echo "mount-test: the mount never came up" >&2
  exit 1
fi

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
# Through this test's own store mount rather than the tree's symlink: the
# symlink points at an absolute /nix/store path, and this test deliberately
# does not mount over the real one.
head -c 1 "$WORK/store/$BASE/bin/$NAME" > /dev/null
downloads_after_reading="$(ls "$WORK/cache/store" | wc -l)"
if [ "$downloads_after_reading" -ne 1 ]; then
  echo "mount-test: reading a file fetched $downloads_after_reading path(s)" >&2
  exit 1
fi

kill "$MOUNT_PID" 2>/dev/null || true
echo "ok: looking is free, reading costs one path"

# ---------------------------------------------------------------------------
# A passthrough store, which needs no network and no index.
#
# Both of the bugs this section covers were invisible to everything above.
# A store path can be a regular file rather than a directory, and joining an
# empty position onto one appended a trailing separator that made it fail to
# stat. And a daemon asked to stop used to ignore the request entirely,
# leaving its mounts behind.

echo "== a store path that is a file"
PASS="$WORK/passthrough"
mkdir -p "$PASS" "$WORK/store2" "$WORK/tree2"
mkdir -p "$PASS/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-adir/bin"
echo hello > "$PASS/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-adir/bin/cmd"
head -c 4096 /dev/urandom > "$PASS/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-afile.db"

OMNIBIN_DB="$WORK/omnibin.db" "$OMNIBIN" mount \
  --store "$WORK/store2" --tree "$WORK/tree2" --cache-dir "$WORK/cache2" \
  --passthrough "$PASS" &
PASS_PID=$!

for _ in $(seq 1 50); do
  [ -e "$WORK/tree2/README.md" ] && break
  sleep 0.2
done

if [ ! -e "$WORK/tree2/README.md" ]; then
  echo "mount-test: the passthrough mount never came up" >&2
  exit 1
fi

# The directory always worked; the file is the regression.
test "$(stat -c %F "$WORK/store2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-adir")" = directory
kind="$(stat -c %F "$WORK/store2/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-afile.db" 2>&1)"
if [ "$kind" != "regular file" ]; then
  echo "mount-test: a file-shaped store path stats as '$kind'" >&2
  exit 1
fi
test "$(stat -c %s "$WORK/store2/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-afile.db")" -eq 4096
test "$(head -c 16 "$WORK/store2/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-afile.db" | wc -c)" -eq 16
echo "ok: a store path that is a file resolves, stats and reads"

echo "== asking it to stop unmounts"
kill -TERM "$PASS_PID"
for _ in $(seq 1 50); do
  kill -0 "$PASS_PID" 2>/dev/null || break
  sleep 0.2
done

if kill -0 "$PASS_PID" 2>/dev/null; then
  echo "mount-test: the daemon ignored SIGTERM" >&2
  kill -9 "$PASS_PID" 2>/dev/null || true
  exit 1
fi

left="$(mount | grep -c "$WORK/store2" || true)"
if [ "$left" -ne 0 ]; then
  echo "mount-test: $left mount(s) left behind after SIGTERM" >&2
  exit 1
fi
echo "ok: SIGTERM exits and leaves no mount"
