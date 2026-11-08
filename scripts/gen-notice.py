#!/usr/bin/env python3
"""Writes THIRD_PARTY_RUST.md: every Rust crate Relay is built with (outside this repository), its version
and its licence, from `cargo metadata`. Run from the repository root:

    python3 scripts/gen-notice.py            # rewrites THIRD_PARTY_RUST.md
    python3 scripts/gen-notice.py --check    # exits 1 when the file is out of date

Only crates that end up in a build are listed (the dependency graph from the workspace members), not
crates that appear only in a lock file for other platforms.
"""
import json
import subprocess
import sys
from pathlib import Path

OUT = Path("THIRD_PARTY_RUST.md")
HEADER = """# Third-party Rust crates

Relay is licensed MIT OR Apache-2.0 (see LICENSE-MIT and LICENSE-APACHE). It is built with the crates
below, each under the licence shown (an `OR` means the choice is Relay's, `AND` means all apply). Generated
by `scripts/gen-notice.py` from `cargo metadata`; do not edit by hand. The Android app's own libraries
(AndroidX, Compose, Kotlin and others) are Apache-2.0 and are listed in its dependency files.

| Crate | Version | Licence |
|---|---|---|
"""


def render() -> str:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--locked"], text=True, encoding="utf-8"
        )
    )
    members = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    pkgs = {p["id"]: p for p in meta["packages"]}
    seen, stack = set(), list(members)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        stack.extend(d["pkg"] for d in nodes[i]["deps"])
    rows = []
    for i in sorted(seen - members, key=lambda k: (pkgs[k]["name"], pkgs[k]["version"])):
        p = pkgs[i]
        lic = (p.get("license") or "see the crate").replace("|", "\\|")
        rows.append(f"| {p['name']} | {p['version']} | {lic} |")
    return HEADER + "\n".join(rows) + "\n"


def main() -> int:
    text = render()
    if "--check" in sys.argv:
        if not OUT.exists() or OUT.read_text(encoding="utf-8").replace("\r\n", "\n") != text:
            print(f"{OUT} is out of date: run python3 scripts/gen-notice.py")
            return 1
        print(f"{OUT} is up to date")
        return 0
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
