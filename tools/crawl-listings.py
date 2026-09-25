#!/usr/bin/env python3
"""Crawl cache.nixos.org for the file listing of every indexed store path.

Hydra publishes a `.ls` next to every narinfo: the complete NAR listing as
JSON, with each entry's type, size, executable bit and symlink target. That is
the whole metadata half of a Nix store, already served, at about 9 KB
compressed per path. This script fetches one per digest and writes them as
JSON Lines, which is the only artifact omnibin adds to what
nixpkgs-multiverse already publishes.

The crawl is not resumable and does not need to be: 316k requests over
keep-alive connections finish in minutes, and re-running it is cheaper than
any mechanism that would let it resume. Point it at a fresh output file and
let it run again.

Needs brotli and zstandard, because the cache serves `.ls` with
`Content-Encoding: br` for older objects and `zstd` for newer ones:

    nix-shell -p 'python3.withPackages (ps: with ps; [ brotli zstandard ])' \
      --run 'tools/crawl-listings.py --outpaths outpaths-x86_64-linux.json \
             --out data/listings.jsonl.zst'
"""

import argparse
import gzip
import http.client
import json
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import brotli
import zstandard

CACHE_HOST = "cache.nixos.org"

# Every encoding the cache is known to serve a `.ls` with, sent as the request's
# Accept-Encoding. The response's Content-Encoding is only a hint: some objects
# come back as a zstd frame with the header missing or set to identity, so
# decode() sniffs the magic bytes first and falls back to the header.
DECODERS = {
    "br": brotli.decompress,
    "zstd": lambda b: zstandard.ZstdDecompressor().decompress(
        b, max_output_size=MAX_LISTING_BYTES
    ),
    "gzip": gzip.decompress,
    "identity": lambda b: b,
}

# A decompression bound for the zstd decoder, which needs one when the frame
# carries no content size. The largest listing measured across a 400-path
# sample was 3.5 MB of JSON; a 256 MB ceiling is far above anything real and
# still refuses a decompression bomb.
MAX_LISTING_BYTES = 256 * 1024 * 1024

# How long a worker waits on a single request before giving up and retrying.
REQUEST_TIMEOUT_SECONDS = 60

# Attempts per digest. The cache sits behind a CDN that occasionally resets a
# connection under this much parallelism; a retry costs one request and a
# missing listing costs a package.
MAX_ATTEMPTS = 4

# Progress is printed on this cadence rather than per record, so a multi-hour
# run on a headless box leaves a readable log rather than 316k lines.
PROGRESS_EVERY_SECONDS = 15


# Leading bytes of each compressed format, checked before the Content-Encoding
# header. A `.ls` body is JSON, so an opening brace means it arrived as-is.
MAGIC = {
    b"\x28\xb5\x2f\xfd": "zstd",
    b"\x1f\x8b": "gzip",
    b"{": "identity",
}


def decode(body, encoding):
    """Decompress a `.ls` body. Magic bytes win over the declared encoding.

    Brotli has no magic number, so it is what is left when nothing matches and
    the header does not say otherwise.
    """
    for magic, name in MAGIC.items():
        if body.startswith(magic):
            return DECODERS[name](body)

    decoder = DECODERS.get(encoding)
    if decoder is None:
        raise OSError(f"unknown Content-Encoding {encoding!r}")

    # A header claiming identity on a body that is not JSON is a lie the cache
    # tells about some older objects; those are brotli.
    if encoding == "identity":
        return DECODERS["br"](body)

    return decoder(body)


class Fetcher:
    """One HTTPS connection per thread, reused across requests.

    A fresh connection per digest spends more time in TLS than in transfer;
    holding one per worker is what turns this crawl from an hour into minutes.
    """

    def __init__(self):
        self.local = threading.local()

    def connection(self):
        conn = getattr(self.local, "conn", None)
        if conn is not None:
            return conn

        conn = http.client.HTTPSConnection(CACHE_HOST, timeout=REQUEST_TIMEOUT_SECONDS)
        self.local.conn = conn
        return conn

    def drop(self):
        """Discard this thread's connection so the next call dials a new one."""
        conn = getattr(self.local, "conn", None)
        if conn is None:
            return

        try:
            conn.close()
        except OSError:
            pass
        self.local.conn = None

    def get(self, path):
        """GET one path; return (status, decoded body). Body is None on 404."""
        conn = self.connection()
        conn.request(
            "GET",
            path,
            headers={
                "Accept-Encoding": ", ".join(DECODERS),
                "User-Agent": "omnibin-crawl-listings/1",
            },
        )
        resp = conn.getresponse()
        body = resp.read()

        if resp.status == 404:
            return 404, None
        if resp.status != 200:
            raise OSError(f"{path}: HTTP {resp.status}")

        return 200, decode(body, resp.getheader("Content-Encoding", "identity"))


def already_crawled(paths):
    """Every digest an existing crawl file already answered for.

    A listing is content addressed, so one that has been fetched is correct
    forever and there is no reason to fetch it twice. This is what turns a
    daily run from a 233,000 request crawl into a 3,000 request one.
    """
    seen = set()
    for path in paths:
        zstd = subprocess.Popen(["zstd", "-dc", path], stdout=subprocess.PIPE)
        for line in zstd.stdout:
            seen.add(json.loads(line)["d"])
        if zstd.wait() != 0:
            sys.exit(f"crawl-listings: zstd failed reading {path}")

    return seen


def read_digests(args):
    """The digests to crawl, from a multiverse outpaths artifact or a flat list.

    An outpaths file is {"attrs": {attr: {version: [digest, ...]}}}; the same
    digest can be named by several (attr, version) pairs, and a listing is
    per path, so the result is deduplicated and sorted for a stable run.
    """
    if args.digests:
        digests = [line.strip() for line in open(args.digests) if line.strip()]
        return sorted(set(digests))

    doc = json.load(open(args.outpaths))

    # An outpaths entry is [digest] or [digest, store-path name]: the name is
    # carried only where it differs from what the attribute implies. The
    # listing is addressed by digest alone, so only the first element is read.
    digests = {
        entry[0] for versions in doc["attrs"].values() for entry in versions.values()
    }
    return sorted(digests)


def crawl(digests, out, jobs):
    """Fetch every digest's listing and write one JSON object per line.

    Records are written by the main thread as results arrive, so the output
    needs no lock and the worker pool never blocks on I/O to disk. A digest
    the cache does not hold is written as `ok: false` rather than dropped:
    absence is an answer, and re-deriving it costs another request.
    """
    fetcher = Fetcher()
    counts = {"ok": 0, "missing": 0, "malformed": 0, "failed": 0}
    started = time.monotonic()
    last_report = started

    def fetch(digest):
        for attempt in range(MAX_ATTEMPTS):
            try:
                status, body = fetcher.get(f"/{digest}.ls")
            except (OSError, http.client.HTTPException):
                # A reset connection is dead for every later request on this
                # thread, so drop it before backing off.
                fetcher.drop()
                time.sleep(2**attempt)
                continue

            if status == 404:
                return {"d": digest, "ok": False}

            # A small number of published listings decompress to malformed
            # JSON. The bytes on the cache are damaged, and curl's own brotli
            # reproduces the same gap, so this is not a decoder bug. Record the
            # digest as answered-but-unusable rather than aborting the crawl.
            try:
                root = json.loads(body)["root"]
            except (ValueError, KeyError):
                return {"d": digest, "ok": False, "error": "malformed"}

            return {"d": digest, "ok": True, "root": root}

        return {"d": digest, "ok": False, "error": "unreachable"}

    with ThreadPoolExecutor(jobs) as pool:
        for i, record in enumerate(pool.map(fetch, digests), start=1):
            if record.get("error") == "unreachable":
                counts["failed"] += 1
            elif record.get("error") == "malformed":
                counts["malformed"] += 1
            elif record["ok"]:
                counts["ok"] += 1
            else:
                counts["missing"] += 1

            out.write(json.dumps(record, separators=(",", ":")).encode())
            out.write(b"\n")

            now = time.monotonic()
            if now - last_report < PROGRESS_EVERY_SECONDS:
                continue

            rate = i / (now - started)
            remaining = (len(digests) - i) / rate if rate else 0
            print(
                f"{i}/{len(digests)}  {rate:.0f}/s  "
                f"eta {remaining/60:.1f}m  "
                f"ok {counts['ok']} missing {counts['missing']} "
                f"malformed {counts['malformed']} failed {counts['failed']}",
                file=sys.stderr,
                flush=True,
            )
            last_report = now

    return counts, time.monotonic() - started


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    source = ap.add_mutually_exclusive_group(required=True)
    source.add_argument(
        "--outpaths", help="multiverse outpaths-<system>.json to take digests from"
    )
    source.add_argument("--digests", help="file of digests, one per line")
    ap.add_argument("--out", required=True, help="output listings.jsonl.zst")
    ap.add_argument(
        "--have",
        nargs="+",
        default=[],
        help="existing listings files whose digests should not be fetched again",
    )
    ap.add_argument("--jobs", type=int, default=64, help="concurrent requests")
    ap.add_argument("--limit", type=int, help="crawl only the first N digests")
    args = ap.parse_args()

    digests = read_digests(args)

    if args.have:
        seen = already_crawled(args.have)
        digests = [d for d in digests if d not in seen]
        print(f"skipping {len(seen)} digests already crawled", file=sys.stderr)

    if args.limit:
        digests = digests[: args.limit]

    if not digests:
        print("crawl-listings: nothing new to crawl", file=sys.stderr)
        return
    print(f"crawling {len(digests)} listings with {args.jobs} workers", file=sys.stderr)

    # zstd as a subprocess rather than in-process: the writer is the one thing
    # that must keep up with 64 workers, and `zstd -T0` uses every core for it.
    zstd = subprocess.Popen(
        ["zstd", "-T0", "-q", "-o", args.out, "-f"], stdin=subprocess.PIPE
    )
    counts, elapsed = crawl(digests, zstd.stdin, args.jobs)
    zstd.stdin.close()
    if zstd.wait() != 0:
        sys.exit("crawl-listings: zstd failed")

    print(
        f"{args.out}: {counts['ok']} listings, {counts['missing']} not in cache, "
        f"{counts['malformed']} malformed, {counts['failed']} unreachable, "
        f"in {elapsed/60:.1f}m",
        file=sys.stderr,
    )
    if counts["failed"]:
        sys.exit("crawl-listings: some digests never answered; re-run")


if __name__ == "__main__":
    main()
