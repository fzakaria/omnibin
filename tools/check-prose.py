#!/usr/bin/env python3
"""Fail on the house style's banned constructions.

Prose in this repository, including code comments, is held to one rule set so
that nobody has to remember it and no review has to argue about it. The checks
here are the mechanical ones. Everything else is a matter of taste and stays
that way.

    tools/check-prose.py            # the whole tree
    tools/check-prose.py README.md
"""

import argparse
import re
import sys
from pathlib import Path

# Extensions whose prose is held to the rules. Data files are excluded: a JSON
# artifact full of store path names is not prose and would only produce noise.
CHECKED_SUFFIXES = {".md", ".rs", ".py", ".sh", ".nix", ".yml"}

SKIP_DIRS = {".git", ".jj", "target", "data", "result", "__pycache__", ".mypy_cache"}

# The loudest signal of machine written text. Matched case insensitively on a
# word boundary.
BANNED_WORDS = [
    "delve",
    "tapestry",
    "meticulous",
    "pivotal",
    "intricate",
    "interplay",
    "underscore",
    "garner",
    "bolster",
    "vibrant",
    "bustling",
    "multifaceted",
    "seamless",
    "commendable",
    "ever-evolving",
    "realm",
    "testament",
    "showcase",
    "foster",
    "unlock",
    "elevate",
    "embark",
    "robust",
    "crucial",
    "essential",
    "profound",
    "nuanced",
    "holistic",
    "myriad",
    "plethora",
    "leverage",
]

# Connective tics and stock phrases.
BANNED_PHRASES = [
    "it is important to note",
    "it is worth noting",
    "it should be noted",
    "keep in mind that",
    "plays a crucial role",
    "stands as a testament",
    "in the heart of",
    "nestled in",
    "hidden gem",
    "it just works",
    "batteries included",
    "zero config",
    "from the ground up",
    "first-class citizen",
    "game changer",
    "despite these challenges",
    "challenges remain",
    "only time will tell",
    "to be honest",
    "let me be clear",
    "that is the whole point",
    "not just",
    "not only",
]

# An em dash. The style allows them rarely; this repository does without.
EM_DASH = "—"


def offenders(path):
    """Every (line number, rule, line) this file breaks."""
    found = []
    for n, line in enumerate(path.read_text(errors="replace").splitlines(), start=1):
        if EM_DASH in line:
            found.append((n, "em dash", line.strip()))

        lowered = line.lower()
        for phrase in BANNED_PHRASES:
            if phrase in lowered:
                found.append((n, f"phrase {phrase!r}", line.strip()))

        for word in BANNED_WORDS:
            if re.search(rf"\b{re.escape(word)}\b", lowered):
                found.append((n, f"word {word!r}", line.strip()))

    return found


def walk(root):
    for path in sorted(root.rglob("*")):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        if path.is_file() and path.suffix in CHECKED_SUFFIXES:
            yield path


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "paths", nargs="*", help="files to check; the whole tree when omitted"
    )
    args = ap.parse_args()

    root = Path(__file__).resolve().parent.parent
    paths = [Path(p) for p in args.paths] if args.paths else list(walk(root))

    # This file names every banned word in order to check for them, so it is
    # the one file the rules cannot be applied to.
    paths = [p for p in paths if p.resolve() != Path(__file__).resolve()]

    total = 0
    for path in paths:
        for n, rule, line in offenders(path):
            rel = (
                path.resolve().relative_to(root)
                if path.resolve().is_relative_to(root)
                else path
            )
            print(f"{rel}:{n}: {rule}\n    {line}")
            total += 1

    if total:
        print(f"\n{total} violation(s)", file=sys.stderr)
        sys.exit(1)

    print(f"{len(paths)} files checked, no violations")


if __name__ == "__main__":
    main()
