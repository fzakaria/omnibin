#!/usr/bin/env bash
# Downloads every artifact data-pins.json names into data/, verifying each
# against its pinned narHash.
#
# This is the developer's road into the same bytes the flake fetches. Nothing
# in the build calls it: nix/data.nix fetches each file as its own
# fixed-output derivation.
set -euo pipefail

ROOT="${OMNIBIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
DATA="$ROOT/data"
mkdir -p "$DATA/listings"

python3 - "$ROOT/data-pins.json" "$DATA" <<'PY'
import json, subprocess, sys, os

pins_file, data = sys.argv[1:3]
pins = json.load(open(pins_file))
base = pins["baseUrl"]

for name, pin in sorted(pins["files"].items()):
    # Listings shards live in their own directory so shard-listings.py can
    # read what is already published; everything else sits at the top.
    out = os.path.join(data, "listings" if name.startswith("listings-") else "", name)

    if os.path.exists(out):
        have = subprocess.run(
            ["nix", "hash", "path", "--sri", "--type", "sha256", out],
            capture_output=True, text=True, check=True).stdout.strip()
        if have == pin["narHash"]:
            continue

    url = f"{base}/{pin['tag']}/{name}"
    print(f"fetching {name} from {pin['tag']}", file=sys.stderr)
    subprocess.run(["curl", "-sSfL", "-o", out, url], check=True)

    have = subprocess.run(
        ["nix", "hash", "path", "--sri", "--type", "sha256", out],
        capture_output=True, text=True, check=True).stdout.strip()
    if have != pin["narHash"]:
        sys.exit(f"fetch-data: {name} hashed {have}, pinned {pin['narHash']}")
PY
