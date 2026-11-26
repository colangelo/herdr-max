#!/usr/bin/env bash
# Print the next herdr-max build's version and build id as `key=value` lines,
# for both builders: .github/workflows/beta.yml (GitHub) and
# scripts/beta_local.sh (`just release-local`, a Mac of ours).
#
#   scripts/beta_version.sh
#
# Environment: GH_REPO (default colangelo/herdr-max), BETA_ALLOW_SAME_VERSION
# (rebuild the published version, e.g. to finish a half-published build).
# Run from the repository root (reads Cargo.toml and HEAD).
#
# The version is plain semver and is Cargo.toml's: herdr-max has its own
# (0.10.0 first, then patch for a fix and minor for a feature), with no
# -beta.N and no codename since the herdr-beta builds ended at 222
# (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/330). The build id
# is the commit date and short sha, e.g. 20261126-ab12cd3; it only shows in
# `herdr --version` and the release title, for tracing.
#
# `brew upgrade` upgrades on a higher version only, so a published version is
# never built again: bump Cargo.toml for every build.
set -euo pipefail

REPO=${GH_REPO:-colangelo/herdr-max}

BASE=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
case "$BASE" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *)
    echo "Cargo.toml version '${BASE}' is not major.minor.patch" >&2
    exit 1
    ;;
esac

# The published release's version, none when there is no release yet. Any other
# gh failure stops here: reading a network error as "no release" would let an
# old or repeated version through.
if title=$(gh release view beta --repo "$REPO" --json name --jq .name 2>&1); then
  # "Max 0.10.0 (build ...)" now, "Beta 0.9.5-ac-beta.222-bonucci" before.
  LAST=$(sed -n 's/^[A-Za-z]* \([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p' <<<"$title")
  if [ -z "$LAST" ]; then
    echo "the published release title '${title}' has no version" >&2
    exit 1
  fi
else
  case "$title" in
    *"release not found"*) LAST= ;;
    *)
      echo "could not read the published release: ${title}" >&2
      exit 1
      ;;
  esac
fi

if [ -n "$LAST" ] && [ -z "${BETA_ALLOW_SAME_VERSION:-}" ]; then
  FIRST=$(printf '%s\n%s\n' "$LAST" "$BASE" | sort -V | head -1)
  # Equal to the published base, or lower: brew would not move to it. The old
  # beta's base (0.9.5) is below 0.10.0, so the first herdr-max passes.
  if [ "$BASE" = "$LAST" ] || [ "$FIRST" != "$LAST" ]; then
    echo "Cargo.toml says ${BASE}, but ${LAST} is already published: bump the version (brew upgrades on a higher one only)" >&2
    exit 1
  fi
fi

BUILD_ID="$(git show -s --format=%cd --date=format:%Y%m%d HEAD)-$(git rev-parse --short=7 HEAD)"
echo "build_id=${BUILD_ID}"
echo "version=${BASE}"
