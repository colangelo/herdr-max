#!/usr/bin/env bash
# Publish the rolling `beta` prerelease without ever leaving the channel empty
# on a failure. Called by .github/workflows/beta.yml's publish job.
#
#   scripts/beta_publish.sh ASSET...
#
# Environment: GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID, BETA_VERSION,
# TARGET_SHA (the commit being published), SOURCE_REF (what was dispatched),
# DEFAULT_BRANCH (default: master), BETA_NOTES.
#
# Order, and what a failure at each step leaves behind:
#   1. Refuse a commit whose .github/workflows/ differs from the default
#      branch. GITHUB_TOKEN may not create a ref on such a commit (HTTP 403,
#      "Resource not accessible by integration"); see
#      https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/82.
#      -> nothing touched.
#   2. Publish to a staging tag and download the assets back.
#      -> old `beta` untouched; creating the staging tag is the same ref
#         creation the final tag needs, so a refused token fails here.
#   3. Point the `beta` tag at the new commit.
#      -> old `beta` release still serves its assets.
#   4. Delete the old `beta` release, then move the staging release onto
#      `beta`. Only between these two calls is `beta` briefly without a
#      release; if the move fails, the staging release still holds the build
#      and the error says how to finish by hand.
#   5. Verify `beta` serves the new assets, then drop the staging tag.
set -euo pipefail

: "${GITHUB_REPOSITORY:?}" "${GITHUB_RUN_ID:?}" "${BETA_VERSION:?}" "${TARGET_SHA:?}"
DEFAULT_BRANCH=${DEFAULT_BRANCH:-master}
SOURCE_REF=${SOURCE_REF:-$DEFAULT_BRANCH}
BETA_NOTES=${BETA_NOTES:-}
REPO=$GITHUB_REPOSITORY
STAGING="beta-staging-${GITHUB_RUN_ID}"
TITLE="Beta ${BETA_VERSION}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 ASSET..." >&2
  exit 2
fi
ASSETS=("$@")

error() { echo "::error::$*" >&2; }
step() { echo "==> $*"; }

# gh api exits non-zero for any failure; tell "does not exist" apart from the
# rest so a permission or network error is never read as "nothing to delete".
ref_exists() {
  local out
  if out=$(gh api "repos/${REPO}/git/ref/tags/$1" 2>&1); then
    return 0
  fi
  case "$out" in
    *"HTTP 404"*) return 1 ;;
  esac
  error "could not look up tag $1: $out"
  exit 1
}

release_exists() {
  local out
  if out=$(gh api "repos/${REPO}/releases/tags/$1" 2>&1); then
    return 0
  fi
  case "$out" in
    *"HTTP 404"*) return 1 ;;
  esac
  error "could not look up release $1: $out"
  exit 1
}

verify_release_assets() {
  local tag=$1 dir
  dir=$(mktemp -d)
  gh release download "$tag" --repo "$REPO" --dir "$dir"
  for asset in "${ASSETS[@]}"; do
    local name
    name=$(basename "$asset")
    if ! cmp -s "$asset" "$dir/$name"; then
      error "release $tag does not serve $name byte-for-byte"
      exit 1
    fi
  done
  rm -rf "$dir"
}

# The name and blob sha of every file in .github/workflows/ at a commit, read
# from GitHub at publish time so a default branch that moved during the build
# is seen.
workflow_files() {
  gh api "repos/${REPO}/contents/.github/workflows?ref=$1" \
    --jq '.[] | "\(.name) \(.sha)"' | sort
}

step "1/5 check .github/workflows/ against ${DEFAULT_BRANCH}"
target_workflows=$(workflow_files "$TARGET_SHA")
default_workflows=$(workflow_files "$DEFAULT_BRANCH")
if [ "$target_workflows" != "$default_workflows" ]; then
  error "refusing to publish ${TARGET_SHA} (from '${SOURCE_REF}'): its .github/workflows/ differs from ${DEFAULT_BRANCH}, and GITHUB_TOKEN cannot create the beta tag on such a commit (HTTP 403). Dispatch the beta from ${DEFAULT_BRANCH}, or from a ref whose workflows match it. The live beta release was not touched."
  diff <(echo "$default_workflows") <(echo "$target_workflows") >&2 || true
  exit 1
fi

for leftover in $(gh release list --repo "$REPO" --limit 100 --json tagName --jq '.[].tagName | select(startswith("beta-staging-"))'); do
  if [ "$leftover" != "$STAGING" ]; then
    step "remove leftover staging release ${leftover}"
    gh release delete "$leftover" --repo "$REPO" --yes --cleanup-tag
  fi
done

step "2/5 publish to staging tag ${STAGING}"
gh release create "$STAGING" \
  --repo "$REPO" \
  --prerelease \
  --latest=false \
  --target "$TARGET_SHA" \
  --title "${TITLE} (staging)" \
  --notes "$BETA_NOTES" \
  "${ASSETS[@]}"
verify_release_assets "$STAGING"

step "3/5 point tag beta at ${TARGET_SHA}"
if ref_exists beta; then
  gh api --method PATCH "repos/${REPO}/git/refs/tags/beta" \
    -f sha="$TARGET_SHA" -F force=true >/dev/null
else
  gh api --method POST "repos/${REPO}/git/refs" \
    -f ref=refs/tags/beta -f sha="$TARGET_SHA" >/dev/null
fi

step "4/5 swap the beta release"
if release_exists beta; then
  gh release delete beta --repo "$REPO" --yes
fi
if ! gh release edit "$STAGING" --repo "$REPO" --tag beta --title "$TITLE" --notes "$BETA_NOTES"; then
  error "beta has no release: moving ${STAGING} onto the beta tag failed. The new build is still published as ${STAGING}. Finish by hand: gh release edit ${STAGING} --repo ${REPO} --tag beta --title '${TITLE}'"
  exit 1
fi

step "5/5 verify beta and drop the staging tag"
verify_release_assets beta
if ref_exists "$STAGING"; then
  gh api --method DELETE "repos/${REPO}/git/refs/tags/${STAGING}" >/dev/null
fi
echo "published ${TITLE} at ${TARGET_SHA}"
