#!/usr/bin/env python3
"""Diagnostics for a built index: the README status block and release notes.

One source for both, because a release whose notes disagree with the README is
a release nobody can check. Everything here is read back out of the database
that was actually built, never passed in.

    tools/status.py --db data/omnibin-*.db --readme
    tools/status.py --db data/omnibin-*.db --notes --tag data-20260925

One database per system, and every system published gets its own line.
"""

import argparse
import os
import sqlite3
import sys
from datetime import datetime, timezone

# The README carries the block between these, so a run can rewrite it without
# touching anything a person wrote around it.
BEGIN = "<!-- BEGIN index-status -->"
END = "<!-- END index-status -->"


def counts(db):
    """Every number the status block and the release notes quote."""
    row = lambda sql: db.execute(sql).fetchone()[0]

    return {
        "system": db.execute("SELECT value FROM meta WHERE key='system'").fetchone()[0],
        "names": row("SELECT count(*) FROM latest"),
        "bins": row("SELECT count(*) FROM bins"),
        "pkgs": row("SELECT count(*) FROM pkgs"),
        "attrs": row("SELECT count(DISTINCT attr) FROM pkgs"),
        "paths": row("SELECT count(*) FROM paths"),
        "listed": row("SELECT count(*) FROM paths WHERE has_listing = 1"),
        "bytes": row("SELECT coalesce(sum(nar_size), 0) FROM paths"),
        "oldest": row("SELECT min(last_seen) FROM pkgs WHERE last_seen IS NOT NULL"),
        "newest": row("SELECT max(last_seen) FROM pkgs WHERE last_seen IS NOT NULL"),
        # The first date whose packages have commands anybody can name. Paths
        # older than this are addressable and carry no published listing.
        "namedFrom": row(
            """SELECT min(p.last_seen) FROM pkgs p JOIN paths s USING (digest)
                WHERE s.has_listing = 1 AND p.last_seen IS NOT NULL"""
        ),
    }


def widest(db, limit):
    """The packages shipping the most executables, as a sanity check.

    A package with five hundred binaries is not a bug, it is TeX, and seeing
    the usual suspects at the top is how you notice when a run has gone wrong.
    """
    return db.execute(
        """SELECT attr, version, count(*) c FROM bins
            GROUP BY attr, version ORDER BY c DESC LIMIT ?""",
        (limit,),
    ).fetchall()


def most_versioned(db, limit):
    """The executable names with the most versions behind them."""
    return db.execute(
        "SELECT name, count(*) c FROM bins GROUP BY name ORDER BY c DESC LIMIT ?",
        (limit,),
    ).fetchall()


def readme_block(counts_by_system, tag):
    """The lines that go between the markers in README.md."""
    lines = [
        "",
        "| system | executables | `name@version` | package versions | store paths |",
        "| --- | --- | --- | --- | --- |",
    ]
    for c in counts_by_system:
        lines.append(
            f"| `{c['system']}` | {c['names']:,} | {c['bins']:,} | "
            f"{c['pkgs']:,} | {c['paths']:,} |"
        )

    first = counts_by_system[0]
    total = sum(c["bytes"] for c in counts_by_system) / 1e12

    # Two date ranges rather than one, because they are different claims and
    # stating only the wider one reads as a promise the index cannot keep.
    lines += [
        "",
        f"Store paths from {first['oldest']} to {first['newest']}, "
        f"{total:,.1f} TB unpacked. Commands are nameable from "
        f"{first['namedFrom']} on, which is when Hydra started publishing "
        f"file listings. Built from nixpkgs-multiverse `{tag}`.",
        "",
    ]
    return "\n".join(lines)


def release_notes(dbs, counts_by_system, tag, multiverse_tag):
    """The body of a data release, covering every system in the cut."""
    systems = ", ".join(f"`{c['system']}`" for c in counts_by_system)
    lines = [
        "File listings crawled from cache.nixos.org, and the index built from them.",
        "",
        f"Built from nixpkgs-multiverse `{multiverse_tag}` for {systems}.",
        "",
        "## What is in this cut",
        "",
        "| | " + " | ".join(f"`{c['system']}`" for c in counts_by_system) + " |",
        "| --- |" + " --- |" * len(counts_by_system),
    ]

    rows = [
        ("executables on `PATH`", lambda c: f"{c['names']:,}"),
        ("`name@version` forms", lambda c: f"{c['bins']:,}"),
        ("package versions", lambda c: f"{c['pkgs']:,}"),
        ("attributes", lambda c: f"{c['attrs']:,}"),
        ("store paths addressable", lambda c: f"{c['paths']:,}"),
        ("of those, with a published listing", lambda c: f"{c['listed']:,}"),
        ("unpacked bytes behind them", lambda c: f"{c['bytes'] / 1e12:,.1f} TB"),
        ("dates covered", lambda c: f"{c['oldest']} to {c['newest']}"),
    ]
    for label, render in rows:
        lines.append(
            f"| {label} | " + " | ".join(render(c) for c in counts_by_system) + " |"
        )

    db, c = dbs[0], counts_by_system[0]
    lines += [
        "",
        f"## Widest packages, on `{c['system']}`",
        "",
        "The packages shipping the most executables. TeX and Kaldi at the top is",
        "what a healthy run looks like.",
        "",
        "| package | executables |",
        "| --- | --- |",
    ]
    lines += [f"| `{a}@{v}` | {n:,} |" for a, v, n in widest(db, 5)]
    lines += [
        "",
        f"## Most versioned names, on `{c['system']}`",
        "",
        "| executable | versions |",
        "| --- | --- |",
    ]
    lines += [f"| `{name}` | {n:,} |" for name, n in most_versioned(db, 5)]
    lines += [
        "",
        "## Using it",
        "",
        "Nothing here is fetched by hand. `data-pins.json` in the repository names",
        "this tag and the hash of every asset on it, and the flake fetches them.",
        "",
        "```console",
        "$ nix run github:fzakaria/omnibin",
        "```",
    ]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", required=True, nargs="+", help="one database per system")
    ap.add_argument("--readme", action="store_true", help="rewrite README.md in place")
    ap.add_argument("--notes", action="store_true", help="print release notes")
    ap.add_argument("--tag", default="", help="the omnibin release tag being cut")
    ap.add_argument(
        "--multiverse-tag",
        default="",
        help="the multiverse cut the index was built from; read from data/multiverse/TAG when omitted",
    )
    args = ap.parse_args()

    dbs = [sqlite3.connect(f"file:{path}?mode=ro", uri=True) for path in args.db]
    counts_by_system = sorted((counts(db) for db in dbs), key=lambda c: c["system"])

    multiverse_tag = args.multiverse_tag
    if not multiverse_tag:
        tag_file = os.path.join(os.path.dirname(args.db[0]), "multiverse", "TAG")
        multiverse_tag = (
            open(tag_file).read().strip() if os.path.exists(tag_file) else "unknown"
        )

    if args.notes:
        print(release_notes(dbs, counts_by_system, args.tag, multiverse_tag))
        return

    if not args.readme:
        sys.exit("status: pass --readme or --notes")

    readme = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(args.db[0]))), "README.md"
    )
    text = open(readme).read()
    if BEGIN not in text or END not in text:
        sys.exit(f"status: {readme} has no index-status markers")

    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)

    # Build the whole file before opening it for writing. Opening first means
    # a failure anywhere in readme_block leaves an empty README behind, which
    # is how this script once deleted the one it was updating.
    updated = head + BEGIN + readme_block(counts_by_system, multiverse_tag) + END + tail
    with open(readme, "w") as f:
        f.write(updated)
    print(f"{readme}: status block updated")


if __name__ == "__main__":
    main()
