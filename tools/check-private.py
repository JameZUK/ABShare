#!/usr/bin/env python3
"""Refuses commits and pushes that contain private strings (SPEC §14).

The list of private strings lives in the gitignored file dev/local.private-patterns.txt
(one per line: real hostnames, addresses, library titles, folder names, emails...). This
script itself contains none of them.

usage:
  tools/check-private.py staged     # the staged diff (pre-commit hook)
  tools/check-private.py range A..B # commits about to be pushed (pre-push hook)
  tools/check-private.py all        # the whole history
"""
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent.parent
pfile = root / "dev" / "local.private-patterns.txt"
if not pfile.exists():
    print("check-private: no dev/local.private-patterns.txt; nothing to check against", file=sys.stderr)
    sys.exit(0)
patterns = [p.strip().lower() for p in pfile.read_text().splitlines() if len(p.strip()) >= 5]

mode = sys.argv[1] if len(sys.argv) > 1 else "staged"
if mode == "staged":
    text = subprocess.run(["git", "diff", "--cached", "-U0"], cwd=root, capture_output=True, text=True).stdout
    text += subprocess.run(["git", "config", "user.email"], cwd=root, capture_output=True, text=True).stdout
elif mode == "range":
    rng = sys.argv[2]
    text = subprocess.run(["git", "log", "-p", "--format=%an %ae %cn %ce%n%B", rng], cwd=root, capture_output=True, text=True).stdout
else:
    text = subprocess.run(["git", "log", "--all", "-p", "--format=%an %ae %cn %ce%n%B"], cwd=root, capture_output=True, text=True).stdout

low = text.lower()


def found(p):
    # Plain words ("Tide") match whole words only, so "riptides" or "lockfileVersion"-style
    # identifiers are not hits;
    # anything with digits or punctuation (addresses, ASINs, paths) matches anywhere.
    if re.fullmatch(r"[a-z ]+", p):
        return re.search(r"(?<![a-z0-9])" + re.escape(p) + r"(?![a-z0-9])", low) is not None
    return p in low


hits = sorted({p for p in patterns if found(p)})
if hits:
    print("check-private: refusing, private strings found:", file=sys.stderr)
    for h in hits[:50]:
        print(f"  {h}", file=sys.stderr)
    sys.exit(1)
print(f"check-private: clean ({len(patterns)} private strings checked)")
