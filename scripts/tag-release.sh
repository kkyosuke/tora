#!/usr/bin/env bash
set -euo pipefail
: "${RELEASE_TAG:?}"
python3 scripts/check-release.py
if git rev-parse --verify "refs/tags/$RELEASE_TAG" >/dev/null 2>&1; then
  test "$(git rev-parse "refs/tags/$RELEASE_TAG^{commit}")" = "$(git rev-parse HEAD)" || {
    echo 'Release tag points at another commit; refusing to overwrite.' >&2
    exit 1
  }
else
  git tag "$RELEASE_TAG" HEAD
  git push origin "refs/tags/$RELEASE_TAG"
fi
