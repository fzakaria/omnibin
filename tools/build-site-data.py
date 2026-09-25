#!/usr/bin/env python3
"""Project the index databases into the files the site fetches.

The site is static, so every question it answers has to be a file it can GET.
Three kinds:

  stats.json            totals, distributions and the derived series the
                        stats view draws. One small file, fetched once.
  names-<system>.json   every bare executable name with how many versions
                        stand behind it. This is the search index.
  bins-<system>/<xx>/   the versions themselves, split by the first two
                        characters of the executable's name.

The split matters. All 882,000 version rows are tens of megabytes, and a
page about `python3` wants one name out of them, so a command page costs one
shard of a few kilobytes rather than the whole table.
"""

import argparse
import json
import os
import sqlite3
from collections import Counter, defaultdict

# How many rows the leaderboards carry. Enough to see the shape, few enough
# to read without scrolling.
TOP_N = 20

# Buckets for the "what does a cold run cost" histogram, in bytes. A package
# is mostly interesting by order of magnitude here, not to three digits.
SIZE_BUCKETS = [
    (0, 1_000_000, "under 1 MB"),
    (1_000_000, 10_000_000, "1 to 10 MB"),
    (10_000_000, 100_000_000, "10 to 100 MB"),
    (100_000_000, 1_000_000_000, "100 MB to 1 GB"),
    (1_000_000_000, None, "over 1 GB"),
]

# Buckets for how many executables a package ships.
# Ranges are half open, [low, high). Spelling them inclusively put every
# single executable package in the "over 100" bucket, because (1, 1) matches
# nothing and bucket() falls through to the last label.
BIN_BUCKETS = [
    (1, 2, "1"),
    (2, 6, "2 to 5"),
    (6, 21, "6 to 20"),
    (21, 101, "21 to 100"),
    (101, None, "over 100"),
]


def shard_of(name):
    """The shard a name belongs to: its first two characters, folded.

    Anything outside [a-z0-9] becomes `_`, so a name starting with `+` or `.`
    still lands in a file whose name is safe on every filesystem and in a URL.
    """
    folded = [c if c.isalnum() and c.isascii() else "_" for c in name[:2].lower()]
    return "".join(folded) or "_"


def bucket(value, buckets):
    """The label of the first bucket `value` falls in."""
    for low, high, label in buckets:
        if value >= low and (high is None or value < high):
            return label

    # Falling through means the buckets do not cover the value, which is a bug
    # in the table rather than a thing to paper over with the last label.
    raise ValueError(f"no bucket for {value}")


def totals(db):
    """The headline numbers for one system."""
    one = lambda sql: db.execute(sql).fetchone()[0]
    return {
        "system": db.execute("SELECT value FROM meta WHERE key='system'").fetchone()[0],
        "names": one("SELECT count(*) FROM latest"),
        "bins": one("SELECT count(*) FROM bins"),
        "pkgs": one("SELECT count(*) FROM pkgs"),
        "attrs": one("SELECT count(DISTINCT attr) FROM pkgs"),
        "paths": one("SELECT count(*) FROM paths"),
        "listed": one("SELECT count(*) FROM paths WHERE has_listing = 1"),
        "bytes": one("SELECT coalesce(sum(nar_size), 0) FROM paths"),
        "firstDate": one("SELECT min(last_seen) FROM pkgs WHERE last_seen IS NOT NULL"),
        "lastDate": one("SELECT max(last_seen) FROM pkgs WHERE last_seen IS NOT NULL"),
        # The first date whose packages have executables anybody can name.
        # Earlier paths are addressable but carry no published listing.
        "namedFrom": one(
            """SELECT min(p.last_seen) FROM pkgs p JOIN paths s USING (digest)
                WHERE s.has_listing = 1 AND p.last_seen IS NOT NULL"""
        ),
        "pkgsNamed": one("""SELECT count(*) FROM pkgs p JOIN paths s USING (digest)
                WHERE s.has_listing = 1"""),
    }


def leaderboards(db):
    """The two tables worth staring at: widest packages, most versioned names."""
    widest = db.execute(
        """SELECT attr, version, count(*) c FROM bins
            GROUP BY attr, version ORDER BY c DESC, attr LIMIT ?""",
        (TOP_N,),
    ).fetchall()
    versioned = db.execute(
        "SELECT name, count(*) c FROM bins GROUP BY name ORDER BY c DESC, name LIMIT ?",
        (TOP_N,),
    ).fetchall()
    return (
        [{"attr": a, "version": v, "bins": c} for a, v, c in widest],
        [{"name": n, "versions": c} for n, c in versioned],
    )


def distributions(db):
    """How many executables a package ships, and how much it costs to fetch."""
    per_package = Counter()
    for (count,) in db.execute("SELECT count(*) FROM bins GROUP BY attr, version"):
        per_package[bucket(count, BIN_BUCKETS)] += 1

    sizes = Counter()
    for (size,) in db.execute(
        """SELECT p.nar_size FROM latest l JOIN paths p USING (digest)
            WHERE p.nar_size IS NOT NULL"""
    ):
        sizes[bucket(size, SIZE_BUCKETS)] += 1

    return (
        [
            {"label": label, "count": per_package.get(label, 0)}
            for _, _, label in BIN_BUCKETS
        ],
        [
            {"label": label, "count": sizes.get(label, 0)}
            for _, _, label in SIZE_BUCKETS
        ],
    )


def per_year(db):
    """How packages changed shape over thirteen years.

    For each year, the packages whose newest build is from that year: how many
    there were, how many distinct executables they carried, and the mean
    executables per package.

    The mean is the interesting series, and it runs the other way from what
    you would guess: about 4 executables per package in 2017 against 3.2 now.
    nixpkgs has been splitting packages up, not fattening them.

    The years before 2017 report no executables at all, and that is upstream
    rather than a gap in the crawl: cache.nixos.org serves a narinfo for those
    paths but no `.ls`, so the commands inside them cannot be named without
    fetching the NAR. The paths are still addressable and still run.
    """
    rows = db.execute(
        """SELECT substr(p.last_seen, 1, 4) AS year, p.attr, p.version, count(b.name) AS bins
             FROM pkgs p LEFT JOIN bins b ON b.attr = p.attr AND b.version = p.version
            WHERE p.last_seen IS NOT NULL
            GROUP BY p.attr, p.version"""
    )

    packages = Counter()
    bins = Counter()
    names = defaultdict(set)
    for year, attr, version, count in rows:
        packages[year] += 1
        bins[year] += count

    for year, name in db.execute("""SELECT substr(p.last_seen, 1, 4), b.name
             FROM bins b JOIN pkgs p ON p.attr = b.attr AND p.version = b.version
            WHERE p.last_seen IS NOT NULL"""):
        names[year].add(name)

    return [
        {
            "year": year,
            "packages": packages[year],
            "bins": bins[year],
            "names": len(names.get(year, ())),
            "binsPerPackage": (
                round(bins[year] / packages[year], 2) if packages[year] else 0
            ),
        }
        for year in sorted(packages)
    ]


def write_names(db, out, system):
    """The search index: every bare name and how many versions it has.

    A list of pairs rather than objects, because at fifty thousand entries the
    key names would be most of the file.
    """
    counts = dict(db.execute("SELECT name, count(*) FROM bins GROUP BY name"))
    names = [
        [name, counts.get(name, 0)]
        for (name,) in db.execute("SELECT name FROM latest ORDER BY name")
    ]

    path = os.path.join(out, f"names-{system}.json")
    with open(path, "w") as f:
        json.dump(names, f, separators=(",", ":"))

    return len(names)


def has_columns(db, table, *names):
    """Whether a table carries these columns.

    A published database is pinned, so a schema change here lands before the
    release that satisfies it does. Rather than fail every consumer in that
    window, the newer fields are treated as optional and the site does without
    them until the pins catch up.
    """
    present = {row[1] for row in db.execute(f"PRAGMA table_info({table})")}
    return all(name in present for name in names)


def write_bins(db, out, system):
    """Every version of every executable, split by the name's first two chars.

    Each row carries what the crawl found inside that store path: how many
    files, directories and symlinks, and which top-level directories. That is
    this project's own data rather than anything restated from the cache, and
    it is what lets a row describe a path without anybody fetching one.
    """
    shape = has_columns(db, "paths", "files", "dirs", "links", "top_dirs")
    columns = (
        "p.files, p.dirs, p.links, p.top_dirs" if shape else "NULL, NULL, NULL, NULL"
    )

    shards = defaultdict(lambda: defaultdict(list))
    for (
        name,
        attr,
        version,
        digest,
        store_name,
        nar_size,
        last_seen,
        files,
        dirs,
        links,
        top_dirs,
    ) in db.execute(
        f"""SELECT b.name, b.attr, b.version, b.digest, p.name, p.nar_size,
                   k.last_seen, {columns}
              FROM bins b
              JOIN paths p ON p.digest = b.digest
              JOIN pkgs k ON k.attr = b.attr AND k.version = b.version
             ORDER BY b.name, b.attr, b.version"""
    ):
        shards[shard_of(name)][name].append(
            [
                attr,
                version,
                digest,
                store_name,
                nar_size,
                last_seen,
                files,
                dirs,
                links,
                top_dirs,
            ]
        )

    directory = os.path.join(out, f"bins-{system}")
    os.makedirs(directory, exist_ok=True)
    for shard, names in shards.items():
        with open(os.path.join(directory, f"{shard}.json"), "w") as f:
            json.dump(names, f, separators=(",", ":"))

    return len(shards)


# How many of a package's commands the search index carries. Enough to say
# what a package gives you without this becoming the whole bins table again.
COMMANDS_IN_INDEX = 8


def write_attrs(db, out, system):
    """The package search index: attribute name to the commands it ships.

    The command index answers "what provides rg". This answers the question
    people actually type, which is "ripgrep", because a package's name and the
    name of the command it installs are routinely different and only one of
    them is on the tin.
    """
    commands = defaultdict(set)
    for attr, name in db.execute("SELECT DISTINCT attr, name FROM bins"):
        commands[attr].add(name)

    index = []
    for attr in sorted(commands):
        names = sorted(commands[attr])
        index.append([attr, len(names), names[:COMMANDS_IN_INDEX]])

    path = os.path.join(out, f"attrs-{system}.json")
    with open(path, "w") as f:
        json.dump(index, f, separators=(",", ":"))

    return len(index)


def write_packages(db, out, system):
    """Which commands each build ships, split by the package's first two chars.

    A command page opens one row at a time and wants to say what else came in
    the same package. That is a fact about the package rather than about the
    command, so it lives in its own shard and is fetched only when a row is
    opened.
    """
    shards = defaultdict(lambda: defaultdict(dict))
    for attr, version, name in db.execute(
        "SELECT attr, version, name FROM bins ORDER BY attr, version, name"
    ):
        shards[shard_of(attr)][attr].setdefault(version, []).append(name)

    directory = os.path.join(out, f"pkgs-{system}")
    os.makedirs(directory, exist_ok=True)
    for shard, attrs in shards.items():
        with open(os.path.join(directory, f"{shard}.json"), "w") as f:
            json.dump(attrs, f, separators=(",", ":"))

    return len(shards)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", required=True, nargs="+", help="one database per system")
    ap.add_argument(
        "--multiverse-tag", default="", help="the cut the index was built from"
    )
    ap.add_argument(
        "--out", required=True, help="directory to write the site data into"
    )
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    stats = {"multiverseTag": args.multiverse_tag, "systems": []}

    for path in sorted(args.db):
        db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        system = db.execute("SELECT value FROM meta WHERE key='system'").fetchone()[0]

        widest, versioned = leaderboards(db)
        per_package, sizes = distributions(db)
        entry = totals(db)
        entry.update(
            {
                "widest": widest,
                "mostVersioned": versioned,
                "binsPerPackage": per_package,
                "sizes": sizes,
                "years": per_year(db),
            }
        )
        stats["systems"].append(entry)

        names = write_names(db, args.out, system)
        attrs = write_attrs(db, args.out, system)
        shards = write_bins(db, args.out, system)
        packages = write_packages(db, args.out, system)
        print(
            f"{system}: {names} names, {attrs} packages, "
            f"{shards} bin shards, {packages} package shards"
        )

    with open(os.path.join(args.out, "stats.json"), "w") as f:
        json.dump(stats, f, separators=(",", ":"))
    print(f"{args.out}/stats.json: {len(stats['systems'])} systems")


if __name__ == "__main__":
    main()
