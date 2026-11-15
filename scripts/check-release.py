#!/usr/bin/env python3
"""Fail before publishing if tag and Cargo package identity differ."""
import json
import os
import subprocess

from release_version import parse_version

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], text=True
))
package = next(p for p in metadata["packages"] if p["name"] == "tora")
tag = os.environ["RELEASE_TAG"]
try:
    if not tag.startswith("v"):
        raise ValueError("missing v prefix")
    parse_version(tag[1:])
except ValueError:
    raise SystemExit("Only stable vMAJOR.MINOR.PATCH tags are supported")
if tag != "v" + package["version"]:
    raise SystemExit(f"Tag {tag} does not match Cargo.toml {package['version']}")
print(f"Verified {tag}")
