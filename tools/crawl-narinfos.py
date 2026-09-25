#!/usr/bin/env python3
"""Fetch narinfos for store paths the multiverse graph does not describe.

The multiverse narinfo crawl walks the closures of the paths it indexes, and a
package's separate `bin` output is not in any of them: `bin` references `out`,
never the other way round. So the digests that hold the commands of jq, git and
a few thousand other packages are exactly the ones nothing has recorded a name,
a NAR url or a size for.

This fetches those few thousand directly. It is a separate tool from
crawl-listings.py because it answers a different question about the same paths,
and because keeping every network fetch in the crawl step is what lets
build-index.py stay a pure projection of files on disk.

    tools/crawl-narinfos.py --digests missing.txt --out data/narinfos.jsonl.zst
"""

import argparse
import http.client
import json
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

CACHE_HOST = "cache.nixos.org"

# A Nix store path digest is this many base-32 characters, and the store path
# name is whatever follows the dash after it.
DIGEST_LEN = 32

REQUEST_TIMEOUT_SECONDS = 30
MAX_ATTEMPTS = 4
PROGRESS_EVERY_SECONDS = 15


class Fetcher:
    """One HTTPS connection per thread, reused across requests."""

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
        conn = getattr(self.local, "conn", None)
        if conn is None:
            return

        try:
            conn.close()
        except OSError:
            pass
        self.local.conn = None

    def get(self, path):
        conn = self.connection()
        conn.request("GET", path, headers={"User-Agent": "omnibin-crawl-narinfos/1"})
        resp = conn.getresponse()
        body = resp.read()

        if resp.status == 404:
            return None
        if resp.status != 200:
            raise OSError(f"{path}: HTTP {resp.status}")

        return body.decode()


def parse(digest, text):
    """The four fields a lazy store needs out of a narinfo."""
    fields = {}
    for line in text.splitlines():
        key, _, value = line.partition(": ")
        fields[key] = value

    store_path = fields.get("StorePath", "")
    name = store_path.rsplit("/", 1)[-1][DIGEST_LEN + 1 :]
    if not name or "URL" not in fields:
        return None

    return {
        "d": digest,
        "ok": True,
        "name": name,
        "nar_url": fields["URL"],
        "nar_size": int(fields.get("NarSize", 0)) or None,
        "file_size": int(fields.get("FileSize", 0)) or None,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--digests", required=True, help="file of digests, one per line")
    ap.add_argument("--out", required=True, help="output narinfos.jsonl.zst")
    ap.add_argument("--jobs", type=int, default=64)
    args = ap.parse_args()

    digests = sorted({line.strip() for line in open(args.digests) if line.strip()})
    print(f"fetching {len(digests)} narinfos with {args.jobs} workers", file=sys.stderr)

    fetcher = Fetcher()
    counts = {"ok": 0, "missing": 0, "failed": 0}

    def fetch(digest):
        for attempt in range(MAX_ATTEMPTS):
            try:
                text = fetcher.get(f"/{digest}.narinfo")
            except (OSError, http.client.HTTPException):
                fetcher.drop()
                time.sleep(2**attempt)
                continue

            if text is None:
                return {"d": digest, "ok": False}

            return parse(digest, text) or {"d": digest, "ok": False}

        return {"d": digest, "ok": False, "error": "unreachable"}

    zstd = subprocess.Popen(
        ["zstd", "-T0", "-q", "-o", args.out, "-f"], stdin=subprocess.PIPE
    )
    started = last_report = time.monotonic()

    with ThreadPoolExecutor(args.jobs) as pool:
        for i, record in enumerate(pool.map(fetch, digests), start=1):
            if record.get("error"):
                counts["failed"] += 1
            elif record["ok"]:
                counts["ok"] += 1
            else:
                counts["missing"] += 1

            zstd.stdin.write(json.dumps(record, separators=(",", ":")).encode())
            zstd.stdin.write(b"\n")

            now = time.monotonic()
            if now - last_report >= PROGRESS_EVERY_SECONDS:
                print(f"{i}/{len(digests)}", file=sys.stderr, flush=True)
                last_report = now

    zstd.stdin.close()
    if zstd.wait() != 0:
        sys.exit("crawl-narinfos: zstd failed")

    elapsed = time.monotonic() - started
    print(
        f"{args.out}: {counts['ok']} narinfos, {counts['missing']} not in cache, "
        f"{counts['failed']} unreachable, in {elapsed:.0f}s",
        file=sys.stderr,
    )
    if counts["failed"]:
        sys.exit("crawl-narinfos: some digests never answered; re-run")


if __name__ == "__main__":
    main()
