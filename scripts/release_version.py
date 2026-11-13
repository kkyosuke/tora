"""Stable version grammar shared by release preparation and tag validation."""
import re


def parse_version(value):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError(f"Expected a stable MAJOR.MINOR.PATCH version: {value}")
    return tuple(map(int, value.split(".")))
