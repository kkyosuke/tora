#!/usr/bin/env python3
"""Detect a root-package version increase in a main push, without publishing."""
import os
from pathlib import Path
import subprocess

from release import version_field
from release_version import parse_version


def release_tag(previous, current, lock):
    old, _ = version_field(previous, "[package]", "tora")
    new, _ = version_field(current, "[package]", "tora")
    locked, _ = version_field(lock, "[[package]]", "tora")
    old_version, new_version = parse_version(old), parse_version(new)
    if locked != new:
        raise ValueError("Cargo.toml and Cargo.lock versions must match")
    if old_version == new_version:
        return None
    if new_version < old_version:
        raise ValueError("Refusing to release a version decrease")
    return f"v{new}"


def main():
    # Compare the complete push range; HEAD^ misses earlier bumps in batch pushes.
    before = os.environ["BEFORE_SHA"]
    previous = subprocess.check_output(["git", "show", f"{before}:Cargo.toml"], text=True)
    tag = release_tag(previous, Path("Cargo.toml").read_text(), Path("Cargo.lock").read_text())
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"changed={'true' if tag else 'false'}\n")
        if tag:
            output.write(f"tag={tag}\n")
    print(f"Release {tag}" if tag else "Version unchanged; no release")


if __name__ == "__main__":
    main()
