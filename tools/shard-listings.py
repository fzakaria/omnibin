#!/usr/bin/env python3
"""Split a listings crawl into release-sized shards.

A file listing is content-addressed: the listing for a digest describes bytes
that cannot change, so a shard that has been published is correct forever.
Shards are therefore cut by crawl generation rather than by content — each run
writes only the digests nobody has published before, into files named for the
day they were crawled, and every earlier shard hashes identically to its pin
and is never uploaded again.

That is the whole delta mechanism. There is no merging, no rewriting and no
cross-file invariant to get wrong; a consumer reads every shard and the union
is the artifact.

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
# once and downloaded many times, so they are worth the slower pass.
SHARD_LEVEL = 19


def published_digests(out_dir):
    """Every digest already covered by a shard in out_dir.

    Read from the shards themselves rather than a side file that could
    disagree with them: the shards are the record of what has been published.
    """
    digests = set()
    for shard in sorted(out_dir.glob("listings-*.jsonl.zst")):
        zstd = subprocess.Popen(["zstd", "-dc", str(shard)], stdout=subprocess.PIPE)
        for line in zstd.stdout:
            digests.add(json.loads(line)["d"])
        if zstd.wait() != 0:
            sys.exit(f"shard-listings: zstd failed reading {shard}")

    return digests


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
    ap.add_argument("--in", dest="source", required=True, help="listings.jsonl.zst")
    ap.add_argument("--out-dir", required=True, help="directory of published shards")
    ap.add_argument("--generation", required=True, help="tag for this cut, e.g. 20260924")
    ap.add_argument("--max-part-bytes", type=int, default=DEFAULT_MAX_PART_BYTES)
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    already = published_digests(out_dir)
    print(f"{len(already)} digests already published", file=sys.stderr)

    writer = PartWriter(out_dir, args.generation, args.max_part_bytes)
    kept = skipped = 0

    zstd = subprocess.Popen(["zstd", "-dc", args.source], stdout=subprocess.PIPE)
    for line in zstd.stdout:
        record = json.loads(line)

        # A digest somebody already published is not written again, and neither
        # is one the crawl could not get a usable listing for: absence is
        # recoverable by re-crawling, a wrong row is not.
        if record["d"] in already:
            skipped += 1
            continue
        if not record.get("ok"):
            skipped += 1
            continue

        writer.write(line)
        kept += 1

    if zstd.wait() != 0:
        sys.exit(f"shard-listings: zstd failed reading {args.source}")
    writer.close()

    for path in writer.written:
        print(f"{path}  {os.path.getsize(path)/1e6:.1f} MB")
    print(f"{kept} new listings, {skipped} skipped", file=sys.stderr)


if __name__ == "__main__":
    main()
