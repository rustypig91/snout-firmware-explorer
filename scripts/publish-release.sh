#!/usr/bin/env bash
# Keep incomplete asset uploads private; publication must be the final operation.
set -euo pipefail

tag=${1:?Usage: publish-release.sh TAG ASSET_DIRECTORY}
asset_directory=${2:?Usage: publish-release.sh TAG ASSET_DIRECTORY}
: "${GH_REPO:?Set GH_REPO to the release repository}"

if [[ ! -d "$asset_directory" ]]; then
  echo "Release asset directory is missing: $asset_directory" >&2
  exit 1
fi
mapfile -d '' assets < <(find "$asset_directory" -type f -print0 | sort -z)
if (( ${#assets[@]} == 0 )); then
  echo 'No release assets were downloaded.' >&2
  exit 1
fi

if is_draft=$(gh release view "$tag" --repo "$GH_REPO" --json isDraft --jq .isDraft); then
  if [[ "$is_draft" != true ]]; then
    echo "Release $tag is already published; refusing to replace its assets." >&2
    exit 1
  fi
else
  gh release create "$tag" --repo "$GH_REPO" --verify-tag --draft \
    --title "Rusty's Snout - Firmware Explorer $tag" \
    --notes 'See the assets to download this version.' --generate-notes
fi

# Retry incomplete draft uploads without exposing partially uploaded releases.
gh release upload "$tag" "${assets[@]}" --repo "$GH_REPO" --clobber
gh release edit "$tag" --repo "$GH_REPO" --draft=false --latest
