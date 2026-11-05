#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Print a CycloneDX 1.5 software bill of materials for Cargo.lock.

Every crate in the lock file becomes a component with its version, package
URL and, for registry crates, the SHA-256 recorded in the lock. The output
depends only on Cargo.lock and the timestamp given by --timestamp (ISO 8601
UTC), so the same inputs give the same bytes.

Usage: scripts/sbom.py --timestamp 2026-11-05T00:00:00Z > vernier-0.1.0.cdx.json
"""

import argparse
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--timestamp", required=True, help="ISO 8601 UTC, e.g. 2026-11-05T00:00:00Z")
    args = ap.parse_args()
    if not args.timestamp.endswith("Z"):
        print("[ERROR] --timestamp must be UTC with a Z suffix", file=sys.stderr)
        return 2
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    workspace = (ROOT / "Cargo.toml").read_text()
    version = next(
        line.split('"')[1] for line in workspace.splitlines() if line.startswith("version") and '"' in line
    )
    components = []
    for pkg in sorted(lock["package"], key=lambda p: (p["name"], p["version"])):
        comp = {
            "type": "library",
            "bom-ref": f"pkg:cargo/{pkg['name']}@{pkg['version']}",
            "name": pkg["name"],
            "version": pkg["version"],
            "purl": f"pkg:cargo/{pkg['name']}@{pkg['version']}",
        }
        if "checksum" in pkg:
            comp["hashes"] = [{"alg": "SHA-256", "content": pkg["checksum"]}]
        if "source" not in pkg:
            comp["licenses"] = [{"license": {"id": "GPL-3.0-or-later"}}]
        components.append(comp)
    bom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "timestamp": args.timestamp,
            "component": {
                "type": "application",
                "name": "vernier",
                "version": version,
                "purl": f"pkg:cargo/vernier@{version}",
                "licenses": [{"license": {"id": "GPL-3.0-or-later"}}],
            },
        },
        "components": components,
    }
    json.dump(bom, sys.stdout, indent=1)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
