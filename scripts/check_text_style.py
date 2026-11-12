#!/usr/bin/env python3
"""Fail if any tracked text file contains typographic characters we avoid."""
import subprocess
import sys

FORBIDDEN = {
    "—": "U+2014",
    "–": "U+2013",
    "→": "U+2192",
    "…": "U+2026",
    "“": "U+201C",
    "”": "U+201D",
    "‘": "U+2018",
    "’": "U+2019",
}


def tracked_files():
    out = subprocess.run(
        ["git", "ls-files", "-z"], check=True, capture_output=True
    ).stdout
    return [p.decode("utf-8") for p in out.split(b"\0") if p]


def main(paths):
    failed = False
    for path in paths:
        try:
            with open(path, "rb") as f:
                data = f.read()
        except OSError:
            continue
        if b"\0" in data:
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            for ch, name in FORBIDDEN.items():
                if ch in line:
                    print(f"{path}:{lineno}: forbidden character {name}")
                    failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    args = sys.argv[1:]
    sys.exit(main(args if args else tracked_files()))
