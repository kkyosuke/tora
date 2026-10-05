#!/usr/bin/env python3
"""Fail before publishing if tag and Cargo package identity differ."""
import json
import os
import re
import subprocess

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], text=True
))
package = next(p for p in metadata["packages"] if p["name"] == "tora")
tag = os.environ["RELEASE_TAG"]
if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
    raise SystemExit("Only stable vMAJOR.MINOR.PATCH tags are supported")
if tag != "v" + package["version"]:
    raise SystemExit(f"Tag {tag} does not match Cargo.toml {package['version']}")
print(f"Verified {tag}")
