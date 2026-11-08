#!/usr/bin/env bash
# Downloads the nixpkgs-multiverse artifacts omnibin builds its index from.
#
# omnibin adds one artifact of its own, the file listings, and takes
# everything else from multiverse's published data releases. Taking them from
# a release rather than from a checkout matters: a working tree carries
# intermediate files from whatever ran last, and joining a fresh outpaths file
# against a stale narinfo graph silently drops every package whose digest the
# graph has not seen yet.
#
# Multiverse's dated tags are deltas, not snapshots. A cut uploads only the
# files whose bytes moved since the previous cut, and data-pins.json in the
# multiverse repo records, per file, the tag that holds the current bytes and
# their narHash. So the pins file, not the newest tag, is what names a
# complete set: on a day the linux outs maps did not change they are absent
# from that day's tag and pinned to the day before.
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
#   tools/fetch-multiverse.sh           # the pins on multiverse's main
#   tools/fetch-multiverse.sh <ref>     # the pins at a multiverse commit
set -euo pipefail

ROOT="${OMNIBIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
OUT="$ROOT/data/multiverse"
REPO="fzakaria/nixpkgs-multiverse"
SYSTEM="${OMNIBIN_SYSTEM:-x86_64-linux}"
PINS_PATH="data-pins.json"

# The multiverse commit whose pins describe the cut to fetch. main is the
# newest cut; update-index.yml commits the repointed pins in the same run that
# uploads the assets. The rolling prerelease is deliberately not used: its
# bytes change under you, and an index built from them could not be
# reproduced.
REF="${1:-main}"

mkdir -p "$OUT"

PINS=$(mktemp)
trap 'rm -f "$PINS"' EXIT
gh api -H "Accept: application/vnd.github.raw" \
  "repos/$REPO/contents/$PINS_PATH?ref=$REF" > "$PINS"
if [ ! -s "$PINS" ]; then
  echo "fetch-multiverse: could not read $PINS_PATH at $REF from $REPO" >&2
  exit 1
fi

echo "fetch-multiverse: $REPO@$REF pins -> $OUT"

# One download per pinned file, each from the tag the pins name and checked
# against its narHash. A file already here with the right hash is skipped, so
# the second system's run only fetches its own two per-system files.
TAG=$(python3 - "$PINS" "$OUT" "$SYSTEM" <<'PY'
import json, os, subprocess, sys

pins_file, out, system = sys.argv[1:4]
pins = json.load(open(pins_file))
base = pins["baseUrl"]


def nar_hash(path):
    return subprocess.run(
        ["nix", "hash", "path", "--sri", "--type", "sha256", path],
        capture_output=True, text=True, check=True).stdout.strip()


wanted = {
    name: pin
    for name, pin in pins["files"].items()
    if name.startswith("info-indexed-")
    or name in (f"outpaths-{system}.json", f"outs-{system}.json")
}
if not wanted:
    sys.exit(f"fetch-multiverse: the pins name no artifacts for {system}")

for name, pin in sorted(wanted.items()):
    path = os.path.join(out, name)

    if os.path.exists(path) and nar_hash(path) == pin["narHash"]:
        continue

    url = f"{base}/{pin['tag']}/{name}"
    print(f"fetching {name} from {pin['tag']}", file=sys.stderr)
    subprocess.run(["curl", "-sSfL", "-o", path, url], check=True)

    have = nar_hash(path)
    if have != pin["narHash"]:
        sys.exit(f"fetch-multiverse: {name} hashed {have}, pinned {pin['narHash']}")

# The newest tag any wanted file lives on is the cut this set describes;
# everything pinned to an older tag is unchanged since then.
print(max(pin["tag"] for pin in wanted.values()))
print(f"fetch-multiverse: {len(wanted)} artifacts for {system}", file=sys.stderr)
PY
)

# The cut the index was built from, so a database can be traced back to the
# multiverse data it describes.
echo "$TAG" > "$OUT/TAG"
echo "fetch-multiverse: cut $TAG"
