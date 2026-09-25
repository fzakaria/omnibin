#!/usr/bin/env python3
"""Diagnostics for a built index: the README status block and release notes.

One source for both, because a release whose notes disagree with the README is
a release nobody can check. Everything here is read back out of the database
that was actually built, never passed in.

    tools/status.py --db data/omnibin-x86_64-linux.db --readme
    tools/status.py --db data/omnibin-x86_64-linux.db --notes --tag data-20260924
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


def readme_block(c, tag):
    """The lines that go between the markers in README.md."""
    size = c["bytes"] / 1e12
    return "\n".join(
        [
            "",
            f"- **{c['names']:,} executables** on `PATH`, over **{c['pkgs']:,} package "
            f"versions** of **{c['attrs']:,} attributes**",
            f"- **{c['bins']:,}** `name@version` forms, addressing "
            f"**{c['paths']:,}** store paths and {size:,.1f} TB of unpacked bytes",
            f"- {c['oldest']} to {c['newest']}, for `{c['system']}`, "
            f"built from nixpkgs-multiverse `{tag}`",
            "",
        ]
    )


def release_notes(db, c, tag, multiverse_tag):
    """The body of a data release."""
    lines = [
        f"File listings crawled from cache.nixos.org, and the index built from them.",
        "",
        f"Built from nixpkgs-multiverse `{multiverse_tag}` for `{c['system']}`.",
        "",
        "## What is in this cut",
        "",
        f"| | |",
        f"| --- | --- |",
        f"| executables on `PATH` | {c['names']:,} |",
        f"| `name@version` forms | {c['bins']:,} |",
        f"| package versions | {c['pkgs']:,} |",
        f"| attributes | {c['attrs']:,} |",
        f"| store paths addressable | {c['paths']:,} |",
        f"| of those, with a published listing | {c['listed']:,} |",
        f"| unpacked bytes behind them | {c['bytes'] / 1e12:,.1f} TB |",
        f"| dates covered | {c['oldest']} to {c['newest']} |",
        "",
        "## Widest packages",
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
        "## Most versioned names",
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
    ap.add_argument("--db", required=True)
    ap.add_argument("--readme", action="store_true", help="rewrite README.md in place")
    ap.add_argument("--notes", action="store_true", help="print release notes")
    ap.add_argument("--tag", default="", help="the omnibin release tag being cut")
    ap.add_argument(
        "--multiverse-tag",
        default="",
        help="the multiverse cut the index was built from; read from data/multiverse/TAG when omitted",
    )
    args = ap.parse_args()

    db = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    c = counts(db)

    multiverse_tag = args.multiverse_tag
    if not multiverse_tag:
        tag_file = os.path.join(os.path.dirname(args.db), "multiverse", "TAG")
        multiverse_tag = (
            open(tag_file).read().strip() if os.path.exists(tag_file) else "unknown"
        )

    if args.notes:
        print(release_notes(db, c, args.tag, multiverse_tag))
        return

    if not args.readme:
        sys.exit("status: pass --readme or --notes")

    readme = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(args.db))), "README.md"
    )
    text = open(readme).read()
    if BEGIN not in text or END not in text:
        sys.exit(f"status: {readme} has no index-status markers")

    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)
    open(readme, "w").write(head + BEGIN + readme_block(c, multiverse_tag) + END + tail)
    print(f"{readme}: status block updated")


if __name__ == "__main__":
    main()
