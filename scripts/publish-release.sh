#!/usr/bin/env bash
set -euo pipefail
: "${RELEASE_TAG:?}" "${GH_REPO:?}"
# Resume incomplete drafts, but never replace published assets.
if draft=$(gh release view "$RELEASE_TAG" --json isDraft --jq .isDraft); then
  if [ "$draft" = false ]; then
    echo "Release $RELEASE_TAG is already published."
    exit 0
  fi
else
  gh release create "$RELEASE_TAG" --verify-tag --draft --title "$RELEASE_TAG" --generate-notes
fi
gh release upload "$RELEASE_TAG" dist/* --clobber
gh release edit "$RELEASE_TAG" --draft=false --latest
