#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Print a CycloneDX 1.5 software bill of materials for Cargo.lock.

Every crate in the lock file becomes a component with its version, package
URL and, for registry crates, the SHA-256 recorded in the lock. The output
depends only on Cargo.lock, the workspace Cargo.toml files and the timestamp
given by --timestamp (ISO 8601 UTC), so the same inputs give the same bytes.

Usage: scripts/sbom.py --timestamp 2026-11-05T00:00:00Z > vernier-0.1.0.cdx.json
"""

import argparse
import datetime
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def workspace_licenses():
    """Map each workspace crate's name to its declared SPDX license."""
    root = tomllib.loads((ROOT / "Cargo.toml").read_text())
    inherited = root.get("workspace", {}).get("package", {}).get("license")
    found = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        pkg = tomllib.loads(manifest.read_text()).get("package", {})
        lic = pkg.get("license")
        if isinstance(lic, dict) and lic.get("workspace"):
            lic = inherited
        if pkg.get("name") and isinstance(lic, str):
            found[pkg["name"]] = lic
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--timestamp", required=True, help="ISO 8601 UTC, e.g. 2026-11-05T00:00:00Z")
    args = ap.parse_args()
    try:
        datetime.datetime.strptime(args.timestamp, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        print("[ERROR] --timestamp must be YYYY-MM-DDTHH:MM:SSZ (UTC)", file=sys.stderr)
        return 2
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    root = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = root["workspace"]["package"]["version"]
    licenses = workspace_licenses()
    names = [(p["name"], p["version"]) for p in lock["package"]]
    if len(names) != len(set(names)):
        print("[ERROR] Cargo.lock lists the same crate name and version twice", file=sys.stderr)
        return 1
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
        if "source" not in pkg and pkg["name"] in licenses:
            comp["licenses"] = [{"license": {"id": licenses[pkg["name"]]}}]
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
