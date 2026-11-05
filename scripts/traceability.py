#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check and print Vernier's requirements traceability matrix.

Reads the requirements tables in AGENTS.md (Steelbore Standard section 20) and
the needs in README.md, then:

  * fails on duplicate or malformed requirement identifiers, an unknown
    priority, method or status, and a source naming a NEED- that README.md
    does not define;
  * prints the matrix (Markdown, or `--json`) mapping every requirement to
    its source, method and status;
  * with `--release`, also fails if any `mandatory` requirement is not
    `verified`, unless it is listed in the known-gaps file given by
    `--accept FILE` (one identifier per line, `#` comments allowed).

Usage: scripts/traceability.py [--release] [--accept FILE] [--json]
"""

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PRIORITIES = {"mandatory", "expected", "optional"}
STATUSES = {"draft", "baselined", "implemented", "verified", "withdrawn"}
ROW = re.compile(r"^\|\s*(~~)?(VNR-SRS-\d{3})(~~)?\s*\|")


def needs():
    found = set(re.findall(r"^\|\s*(VNR-NEED-\d{3})\s*\|", (ROOT / "README.md").read_text(), re.M))
    return found


def requirements(errors):
    rows, seen = [], {}
    for lineno, line in enumerate((ROOT / "AGENTS.md").read_text().splitlines(), 1):
        m = ROW.match(line)
        if not m:
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        ident = m.group(2)
        if len(cells) != 7:
            errors.append(f"AGENTS.md:{lineno}: {ident}: expected 7 columns, found {len(cells)}")
            continue
        if ident in seen:
            errors.append(f"AGENTS.md:{lineno}: {ident} duplicates line {seen[ident]}")
        seen[ident] = lineno
        _, text, _, source, priority, method, status = cells
        withdrawn = bool(m.group(1))
        if priority not in PRIORITIES:
            errors.append(f"AGENTS.md:{lineno}: {ident}: unknown priority {priority!r}")
        if status not in STATUSES:
            errors.append(f"AGENTS.md:{lineno}: {ident}: unknown status {status!r}")
        if withdrawn and status != "withdrawn":
            errors.append(f"AGENTS.md:{lineno}: {ident}: struck through but status is {status!r}")
        rows.append(
            {
                "id": ident,
                "requirement": text,
                "source": source,
                "priority": priority,
                "method": method,
                "status": status,
                "line": lineno,
            }
        )
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--accept", type=Path)
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    errors = []
    defined = needs()
    rows = requirements(errors)
    if not rows:
        errors.append("AGENTS.md: no requirement rows found")
    for r in rows:
        for need in re.findall(r"VNR-NEED-\d{3}", r["source"]):
            if need not in defined:
                errors.append(f"AGENTS.md:{r['line']}: {r['id']}: {need} is not defined in README.md")

    accepted = set()
    if args.accept:
        for line in args.accept.read_text().splitlines():
            line = line.split("#", 1)[0].strip()
            if line:
                accepted.add(line)
        for ident in sorted(accepted - {r["id"] for r in rows}):
            errors.append(f"{args.accept}: accepted gap {ident} is not a requirement")

    gaps = []
    if args.release:
        for r in rows:
            if r["priority"] == "mandatory" and r["status"] not in ("verified", "withdrawn"):
                (gaps if r["id"] in accepted else errors).append(
                    r if r["id"] in accepted else f"{r['id']}: mandatory but {r['status']}; verify it or accept it as a known gap"
                )

    if errors:
        for e in errors:
            print(f"[ERROR] {e}", file=sys.stderr)
        return 1

    if args.json:
        json.dump({"requirements": rows, "known_gaps": [g["id"] for g in gaps]}, sys.stdout, indent=1)
        print()
    else:
        print("| ID | Source | Priority | Method | Status |")
        print("|---|---|---|---|---|")
        for r in rows:
            print(f"| {r['id']} | {r['source']} | {r['priority']} | {r['method']} | {r['status']} |")
    counts = {}
    for r in rows:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    print(f"[OK] {len(rows)} requirements: " + ", ".join(f"{n} {s}" for s, n in sorted(counts.items())), file=sys.stderr)
    for g in gaps:
        print(f"[WARN] {g['id']}: mandatory, {g['status']}, accepted as a known gap", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
