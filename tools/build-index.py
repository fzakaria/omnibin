#!/usr/bin/env python3
"""Project the crawled listings and the multiverse index into omnibin.db.

The database is what the filesystem answers from, and it holds exactly three
questions:

  paths   digest -> store-path name, NAR url and sizes, so a path can be
          fetched without asking the cache for its narinfo first
  pkgs    (attribute, version) -> digest, straight from the multiverse index
  bins    executable name -> the packages whose bin/ holds it
  latest  executable name -> the one package a bare name resolves to

Everything but `bins` and `latest` is a reprojection of artifacts
nixpkgs-multiverse already publishes. The listings crawl is the only new data,
and it is what turns "this store path exists" into "this store path has a
`python3` in its bin/".

The file listings themselves are NOT in here. A full file index of the indexed
paths is around 517 million rows; it lives in the sharded listings artifact
and is read by prefix, not by SQL.
"""

import argparse
import glob
import gzip
import json
import os
import sqlite3
import subprocess
import sys

SCHEMA = """
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);

CREATE TABLE paths(
  digest    TEXT PRIMARY KEY,
  name      TEXT NOT NULL,
  nar_url   TEXT NOT NULL,
  nar_size  INTEGER,
  file_size INTEGER,
  has_listing INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE pkgs(
  attr      TEXT NOT NULL,
  version   TEXT NOT NULL,
  digest    TEXT NOT NULL,
  last_seen TEXT,
  PRIMARY KEY (attr, version)
);
CREATE INDEX pkgs_digest ON pkgs(digest);

CREATE TABLE bins(
  name    TEXT NOT NULL,
  attr    TEXT NOT NULL,
  version TEXT NOT NULL,
  digest  TEXT NOT NULL,
  PRIMARY KEY (name, attr, version)
);
CREATE INDEX bins_name ON bins(name);

CREATE TABLE latest(
  name    TEXT PRIMARY KEY,
  attr    TEXT NOT NULL,
  version TEXT NOT NULL,
  digest  TEXT NOT NULL
);
"""

# info-indexed entries are positional: [alive, narSize, fileSize, name, narUrl].
# A dead or never-crawled digest carries nulls in every field but the first.
INFO_ALIVE, INFO_NAR_SIZE, INFO_FILE_SIZE, INFO_NAME, INFO_NAR_URL = range(5)


def load_dates(versions_file, revisions_file):
    """(attr, version) -> the date of the newest revision that shipped it.

    versions.json records an offset into revisions.json rather than a date, so
    the two files have to be read together. The date is what decides which
    package a bare name in /omnibin/bin resolves to.
    """
    revisions = json.load(open(revisions_file))
    versions = json.load(open(versions_file))

    dates = {}
    for attr, entries in versions["attrs"].items():
        for version, offset in entries.items():
            if offset is None:
                continue
            dates[(attr, version)] = revisions[offset]["date"]

    return dates


def load_info(info_files):
    """digest -> (name, nar_url, nar_size, file_size) for every crawled path.

    multiverse publishes the narinfo graph as one shard per period, year files
    for finished years and month files for the current one, so this takes all
    of them and unions the result. Reading only some of them is the difference
    between an index that covers thirteen years and one that covers the last
    few months.

    Entries the crawl found dead carry no name or URL; they are dropped here,
    because a path omnibin cannot fetch is a path it should not advertise.
    """
    info = {}
    for info_file in info_files:
        opener = gzip.open if str(info_file).endswith(".gz") else open
        for digest, entry in json.load(opener(info_file)).items():
            if not entry[INFO_ALIVE] or entry[INFO_NAME] is None:
                continue
            info[digest] = (
                entry[INFO_NAME],
                entry[INFO_NAR_URL],
                entry[INFO_NAR_SIZE],
                entry[INFO_FILE_SIZE],
            )

    return info


def load_narinfos(paths):
    """digest -> the same four fields, for paths crawled directly.

    These are the separate `bin` outputs, which no closure walk reaches. Loaded
    after the graph so that a digest in both takes the freshly fetched answer.
    """
    info = {}
    for path in paths:
        zstd = subprocess.Popen(["zstd", "-dc", path], stdout=subprocess.PIPE)
        for line in zstd.stdout:
            record = json.loads(line)
            if not record.get("ok"):
                continue
            info[record["d"]] = (
                record["name"],
                record["nar_url"],
                record["nar_size"],
                record["file_size"],
            )

        if zstd.wait() != 0:
            sys.exit(f"build-index: zstd failed reading {path}")

    return info


def resolve_info_files(paths):
    """Expand a directory of narinfo shards into the files inside it.

    Either spelling works: a directory, which is what fetch-multiverse.sh
    writes, or the shard files named one by one.
    """
    files = []
    for path in paths:
        if os.path.isdir(path):
            files.extend(
                sorted(glob.glob(os.path.join(path, "info-indexed-*.json.gz")))
            )
            continue
        files.append(path)

    if not files:
        sys.exit("build-index: no info-indexed shards found")

    return files


def bin_entries(root):
    """The names in a listing's top-level bin/ directory.

    Only the top level: a package's executables are what goes on PATH, and
    libexec or share/ helpers are not. Symlinks count, and `python3` is one.
    """
    entries = root.get("entries") or {}
    bin_dir = entries.get("bin") or {}

    # Dotfiles in bin/ are wrapper internals. `.audacious-wrapped` is the real
    # ELF that the `audacious` shell wrapper execs. They are not commands
    # anybody types, and putting them on PATH would be noise.
    return [n for n in (bin_dir.get("entries") or {}) if not n.startswith(".")]


def read_listings(paths):
    """Yield (digest, root) for every listing the crawls actually got.

    Takes several files because a run crawls only what is new, so the current
    state of the listings is the first full crawl plus every delta since.
    """
    for path in paths:
        zstd = subprocess.Popen(["zstd", "-dc", path], stdout=subprocess.PIPE)
        for line in zstd.stdout:
            record = json.loads(line)
            if not record.get("ok"):
                continue
            yield record["d"], record["root"]

        if zstd.wait() != 0:
            sys.exit(f"build-index: zstd failed reading {path}")


def choose_latest(candidates, name):
    """Pick the one package a bare executable name resolves to.

    Four rules, in order, and the first that separates two candidates wins:
    an attribute named after the executable beats one that is not, a newer
    last-seen date beats an older, a shorter attribute name beats a longer,
    and the alphabet settles the rest. `python3` therefore comes from the
    `python3` attribute rather than from `python3Full`, and a bare name always
    resolves to the same package for everyone.
    """
    return min(
        candidates,
        key=lambda c: (
            c["attr"] != name,
            _invert(c["last_seen"]),
            len(c["attr"]),
            c["attr"],
        ),
    )


def _invert(date):
    """Sort a date descending inside an otherwise ascending key."""
    if date is None:
        return ""
    return "".join(chr(ord("9") - int(ch)) if ch.isdigit() else ch for ch in date)


def build(args):
    # The database is a build product, rebuilt from the artifacts every time.
    # Appending to an existing one would leave rows from a previous crawl with
    # nothing to say which cut they came from.
    if os.path.exists(args.out):
        os.remove(args.out)

    db = sqlite3.connect(args.out)
    db.executescript(SCHEMA)

    # The store paths omnibin can serve, which is everything the multiverse
    # narinfo crawl found alive: the indexed packages and every closure member
    # underneath them, since running a package needs both.
    info = load_info(resolve_info_files(args.info_indexed))
    info.update(load_narinfos(args.narinfos))
    db.executemany(
        "INSERT INTO paths(digest, name, nar_url, nar_size, file_size) VALUES (?,?,?,?,?)",
        ((d, *fields) for d, fields in info.items()),
    )
    print(f"paths: {len(info)}", file=sys.stderr)

    # (attribute, version) -> digest, with the date of the newest revision that
    # shipped it, which decides bare-name resolution below.
    dates = load_dates(args.versions, args.revisions)
    outpaths = json.load(open(args.outpaths))
    pkgs = []
    for attr, entries in outpaths["attrs"].items():
        for version, entry in entries.items():
            digest = entry[0]
            if digest not in info:
                continue
            pkgs.append((attr, version, digest, dates.get((attr, version))))
    db.executemany(
        "INSERT INTO pkgs(attr, version, digest, last_seen) VALUES (?,?,?,?)", pkgs
    )
    print(f"pkgs: {len(pkgs)}", file=sys.stderr)

    # Which packages own which digest, so a listing can be attributed back to
    # the (attribute, version) pairs that name it. One digest can be named by
    # several pairs, being the same build shipped under two attributes.
    #
    # A package that splits its binaries into a separate `bin` output has
    # nothing in the main output's bin/, so the sibling map is consulted and
    # that output is registered as belonging to the same package. jq is one of
    # these, and without this it looks like a package that ships no commands.
    siblings = json.load(open(args.outs)) if args.outs else {}
    owners = {}
    for attr, version, digest, last_seen in pkgs:
        owner = {"attr": attr, "version": version, "last_seen": last_seen}
        owners.setdefault(digest, []).append(owner)

        bin_output = siblings.get(digest, {}).get("bin")
        if bin_output:
            owners.setdefault(bin_output, []).append(owner)

    # The one new table: every executable in every indexed package's bin/.
    bins = []
    listed = []
    by_name = {}
    for digest, root in read_listings(args.listings):
        listed.append((digest,))
        for owner in owners.get(digest, ()):
            for name in bin_entries(root):
                bins.append((name, owner["attr"], owner["version"], digest))
                by_name.setdefault(name, []).append({**owner, "digest": digest})

    db.executemany(
        "INSERT OR IGNORE INTO bins(name, attr, version, digest) VALUES (?,?,?,?)", bins
    )
    db.executemany("UPDATE paths SET has_listing = 1 WHERE digest = ?", listed)
    print(
        f"listings: {len(listed)}  bins: {len(bins)}  names: {len(by_name)}",
        file=sys.stderr,
    )

    # What a bare name on PATH resolves to.
    latest = [
        (name, c["attr"], c["version"], c["digest"])
        for name, candidates in by_name.items()
        for c in [choose_latest(candidates, name)]
    ]
    db.executemany(
        "INSERT INTO latest(name, attr, version, digest) VALUES (?,?,?,?)", latest
    )
    print(f"latest: {len(latest)}", file=sys.stderr)

    db.executemany(
        "INSERT INTO meta(key, value) VALUES (?,?)",
        [
            ("system", args.system),
            ("paths", str(len(info))),
            ("pkgs", str(len(pkgs))),
            ("bins", str(len(bins))),
            ("names", str(len(latest))),
        ],
    )
    db.commit()
    db.execute("VACUUM")
    db.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--listings",
        required=True,
        nargs="+",
        help="listings files from crawl-listings.py, the first crawl and every delta",
    )
    ap.add_argument(
        "--outpaths", required=True, help="multiverse outpaths-<system>.json"
    )
    ap.add_argument(
        "--outs", help="multiverse outs-<system>.json, the sibling output map"
    )
    ap.add_argument(
        "--narinfos",
        nargs="+",
        default=[],
        help="narinfos.jsonl.zst from crawl-narinfos.py, for paths the graph misses",
    )
    ap.add_argument(
        "--info-indexed",
        required=True,
        nargs="+",
        help="multiverse info-indexed shards, or the directory holding them",
    )
    ap.add_argument("--versions", required=True, help="multiverse index/versions.json")
    ap.add_argument("--revisions", required=True, help="multiverse revisions.json")
    ap.add_argument("--system", required=True, help="the system these paths are for")
    ap.add_argument("--out", required=True, help="output omnibin.db")
    build(ap.parse_args())


if __name__ == "__main__":
    main()
