#!/usr/bin/env python3
"""Enforce the project language rule: no Cyrillic outside allowed files.

Allowed locations:
- Markdown files with a `.ru.md` suffix (Russian documentation),
- IDE locale files `locales/ru.conf` (Russian UI strings).

Everything else — code, comments, docs, configs, examples — must be English.
"""

import re
import subprocess
import sys

ALLOWED = re.compile(r"(\.ru\.md$|locales/ru\.conf$)")
CYRILLIC = re.compile("[\u0400-\u04FF]")


def main() -> int:
    listing = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard"],
        capture_output=True,
        text=True,
        check=True,
    )
    offenders = []
    for path in listing.stdout.splitlines():
        if ALLOWED.search(path):
            continue
        try:
            with open(path, encoding="utf-8", errors="ignore") as handle:
                text = handle.read()
        except (IsADirectoryError, FileNotFoundError):
            continue
        if CYRILLIC.search(text):
            offenders.append(path)
    if offenders:
        print("Cyrillic found outside allowed files:")
        for path in offenders:
            print(f"  {path}")
        return 1
    print("cyrillic: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
