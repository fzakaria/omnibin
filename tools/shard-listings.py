#!/usr/bin/env python3
"""Split a listings crawl into release-sized shards.

A file listing is content-addressed: the listing for a digest describes bytes
that cannot change, so a shard that has been published is correct forever.
Shards are therefore cut by crawl generation rather than by content. Each run
writes what it was given into files named for the day, and every earlier shard
hashes identically to its pin and is never uploaded again.

Deciding what is new is the crawl's job, not this script's: crawl-listings.py
takes --have and skips digests an earlier crawl already answered for. Scanning
the published shards again here would re-read the whole archive on every run,
which grows without bound, to re-derive something the caller already knew. A
duplicate that slips through costs a few kilobytes and nothing else, because
build-index.py keys on the digest.

There is no merging, no rewriting and no cross-file invariant to get wrong; a
consumer reads every shard and the union is the artifact.

Parts are capped because a GitHub release asset may not exceed 2 GB.

    tools/shard-listings.py --in data/listings.jsonl.zst \\
      --out-dir data/listings --generation 20260924
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

# Well under GitHub's 2 GB asset ceiling, so a part that compresses worse than
# expected still uploads.
DEFAULT_MAX_PART_BYTES = 1_000_000_000

# Compression level for the published shards. The crawl writes at zstd's
# default because it has to keep up with the network; the shards are written
# once and downloaded many times, so they are worth a slower pass. Level 19 on
# twenty five gigabytes of JSON costs hours for a few percent, so this sits at
# the point where the curve flattens.
SHARD_LEVEL = 10


class PartWriter:
    """Writes records into numbered parts, rolling over at the size cap."""

    def __init__(self, out_dir, generation, max_bytes):
        self.out_dir = out_dir
        self.generation = generation
        self.max_bytes = max_bytes
        self.part = 0
        self.proc = None
        self.written = []

    def _open(self):
        path = self.out_dir / f"listings-{self.generation}-{self.part:03d}.jsonl.zst"
        self.proc = subprocess.Popen(
            ["zstd", f"-{SHARD_LEVEL}", "-T0", "-q", "-f", "-o", str(path)],
            stdin=subprocess.PIPE,
        )
        self.path = path
        self.bytes_in = 0

    def write(self, line):
        if self.proc is None:
            self._open()

        self.proc.stdin.write(line)
        self.bytes_in += len(line)

        # Roll over on uncompressed input, checked after the write so a record
        # is never split across two parts.
        if self.bytes_in >= self.max_bytes * 10:
            self.close()
            self.part += 1

    def close(self):
        if self.proc is None:
            return

        self.proc.stdin.close()
        if self.proc.wait() != 0:
            sys.exit("shard-listings: zstd failed writing a part")
        self.written.append(self.path)
        self.proc = None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--in",
        dest="sources",
        required=True,
        nargs="+",
        help="listings files to publish, the first crawl and every delta since",
    )
    ap.add_argument("--out-dir", required=True, help="directory of published shards")
    ap.add_argument(
        "--generation", required=True, help="tag for this cut, e.g. 20260924"
    )
    ap.add_argument("--max-part-bytes", type=int, default=DEFAULT_MAX_PART_BYTES)
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    writer = PartWriter(out_dir, args.generation, args.max_part_bytes)
    kept = skipped = 0

    for source in args.sources:
        zstd = subprocess.Popen(["zstd", "-dc", source], stdout=subprocess.PIPE)
        for line in zstd.stdout:
            record = json.loads(line)

            # A digest somebody already published is not written again, and
            # neither is one the crawl could not get a usable listing for:
            # absence is recoverable by re-crawling, a wrong row is not.
            if not record.get("ok"):
                skipped += 1
                continue

            writer.write(line)
            kept += 1

        if zstd.wait() != 0:
            sys.exit(f"shard-listings: zstd failed reading {source}")

    writer.close()

    for path in writer.written:
        print(f"{path}  {os.path.getsize(path)/1e6:.1f} MB")
    print(f"{kept} new listings, {skipped} skipped", file=sys.stderr)


if __name__ == "__main__":
    main()
