#!/usr/bin/env bash
# Downloads the nixpkgs-multiverse artifacts omnibin builds its index from.
#
# omnibin adds one artifact of its own, the file listings, and takes
# everything else from multiverse's published data release. Taking them from a
# release rather than from a checkout matters: a working tree carries
# intermediate files from whatever ran last, and joining a fresh outpaths file
# against a stale narinfo graph silently drops every package whose digest the
# graph has not seen yet.
#
# Two artifacts, and they answer different questions:
#
#   outpaths-<system>.json   (attribute, version) -> digest. Which packages
#                            exist and which store path each one is.
#   outs-<system>.json       out digest -> its sibling outputs. Packages that
#                            split their binaries into a separate `bin` output
#                            have nothing in the main output's bin/, and jq is
#                            one of them, so without this map they look like
#                            packages that ship no executables.
#   info-indexed-<period>*   digest -> alive, NAR size, download size, store
#                            path name, NAR url. Published as year shards plus
#                            month shards for the current year; together they
#                            are the whole narinfo crawl, closure members
#                            included, which is what lets the filesystem fetch
#                            a path without asking the cache about it first.
#
# Usage:
#   tools/fetch-multiverse.sh                 # newest dated release
#   tools/fetch-multiverse.sh data-20260924   # a specific cut
set -euo pipefail

ROOT="${OMNIBIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
OUT="$ROOT/data/multiverse"
REPO="fzakaria/nixpkgs-multiverse"
SYSTEM="${OMNIBIN_SYSTEM:-x86_64-linux}"

# The newest dated cut, which is what `gh release list` puts first. The rolling
# prerelease carries working state between cuts and is deliberately not used:
# its bytes change under you, and an index built from them could not be
# reproduced.
TAG="${1:-}"
if [ -z "$TAG" ]; then
  TAG=$(gh release list --repo "$REPO" --limit 20 \
    --json tagName --jq '[.[] | select(.tagName | startswith("data-") and (contains("rolling") | not))][0].tagName')
fi

if [ -z "$TAG" ]; then
  echo "fetch-multiverse: could not find a dated release on $REPO" >&2
  exit 1
fi

mkdir -p "$OUT"
echo "fetch-multiverse: $TAG -> $OUT"

# One download per asset, skipping what is already here. Assets on a dated tag
# are immutable by convention, so a file that exists is the right file.
mapfile -t ASSETS < <(gh release view "$TAG" --repo "$REPO" --json assets \
  --jq ".assets[].name | select(startswith(\"info-indexed-\") or . == \"outpaths-$SYSTEM.json\" or . == \"outs-$SYSTEM.json\")")

if [ "${#ASSETS[@]}" -eq 0 ]; then
  echo "fetch-multiverse: $TAG has no artifacts for $SYSTEM" >&2
  exit 1
fi

for name in "${ASSETS[@]}"; do
  if [ -f "$OUT/$name" ]; then
    continue
  fi
  gh release download "$TAG" --repo "$REPO" --pattern "$name" --dir "$OUT"
done

# The tag the index was built from, so a database can be traced back to the
# multiverse cut it describes.
echo "$TAG" > "$OUT/TAG"

echo "fetch-multiverse: ${#ASSETS[@]} artifacts at $TAG"
