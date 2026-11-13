#!/usr/bin/env bash
# Common verification for local development, CI and release builds.
set -euo pipefail
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
cd "${script_dir%/scripts}"
for script in scripts/*.sh; do bash -n "$script"; done
cargo +stable fmt --check
cargo +stable clippy --locked --all-targets -- -D warnings
cargo +stable test --locked
cargo +stable build --locked
python3 -m unittest discover -s scripts/tests -v
