#!/usr/bin/env bash
set -euo pipefail
: "${TARGET:?}" "${ASSET:?}" "${RELEASE_TAG:?}"
python3 scripts/check-release.py
mkdir -p dist
archive="$ASSET.tar.gz"
tar -czf "dist/$archive" -C "target/$TARGET/release" tora
if command -v sha256sum >/dev/null; then
    hash=$(sha256sum "dist/$archive" | awk '{print $1}')
else
    hash=$(shasum -a 256 "dist/$archive" | awk '{print $1}')
fi
printf '%s  %s\n' "$hash" "$archive" > "dist/$archive.sha256"
printf '%s\n' "$RELEASE_TAG" > "dist/$archive.version"
