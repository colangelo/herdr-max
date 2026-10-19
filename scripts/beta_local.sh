#!/usr/bin/env bash
# Build the Apple-silicon beta on this Mac and publish it as the rolling
# `beta`: the GitHub prerelease (scripts/beta_publish.sh) and the herdr-beta
# formula in colangelo/homebrew-tap. Run by `just beta-local`; `just beta`
# (GitHub, both macOS targets) stays the backup.
# https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/235
#
#   scripts/beta_local.sh [REF]     # REF on origin, default master
#
# Environment:
#   BETA_LOCAL_DRY_RUN=1  build and check the binary, publish nothing
#   BETA_LOCAL_STATE      build clone, tap clone and output (default
#                         ~/.local/state/herdr-beta-local); kept between runs
#                         so a rebuild only compiles what changed
#   BETA_LOCAL_LOCK       the shared heavy-build lock (default /tmp/m4m-heavy.lock)
#   BETA_LOCAL_JOBS       cargo jobs (default 6)
#   CODENAME              pin the codename, as in `just beta`
#   ZIG                   zig 0.16 (default the brew zig@0.16 keg when present)
set -euo pipefail

REF=${1:-master}
REPO=colangelo/herdr-max
TAP=colangelo/homebrew-tap
STATE=${BETA_LOCAL_STATE:-$HOME/.local/state/herdr-beta-local}
LOCK=${BETA_LOCAL_LOCK:-/tmp/m4m-heavy.lock}
JOBS=${BETA_LOCAL_JOBS:-6}
DRY_RUN=${BETA_LOCAL_DRY_RUN:-}
TARGET=aarch64-apple-darwin
ASSET=herdr-macos-aarch64
SCRIPTS=$(cd "$(dirname "$0")" && pwd)

step() { echo "==> $*"; }
die() {
  echo "error: $*" >&2
  exit 1
}

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || die "build on an Apple-silicon Mac"
if [ -z "${ZIG:-}" ] && [ -x /opt/homebrew/opt/zig@0.16/bin/zig ]; then
  ZIG=/opt/homebrew/opt/zig@0.16/bin/zig
fi
ZIG=${ZIG:-zig}
case "$("$ZIG" version)" in
  0.16.*) ;;
  *) die "herdr needs zig 0.16; set ZIG (brew install zig@0.16)" ;;
esac

mkdir -p "$STATE"
SRC=$STATE/src
step "1/6 fetch ${REF} from github.com/${REPO}"
if [ ! -d "$SRC/.git" ]; then
  git clone --quiet "https://github.com/${REPO}.git" "$SRC"
fi
git -C "$SRC" fetch --quiet --force origin "+refs/heads/*:refs/remotes/origin/*"
SHA=$(git -C "$SRC" rev-parse --verify "origin/${REF}^{commit}" 2>/dev/null) ||
  die "${REF} is not a branch on github.com/${REPO}; push it first"
# A detached checkout only rewrites the files that changed, so cargo keeps
# everything else built.
git -C "$SRC" checkout --quiet --force --detach "$SHA"
git -C "$SRC" clean -fdq -e target

cd "$SRC"
TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
ids=$(GH_REPO=$REPO "$SCRIPTS/beta_version.sh")
BUILD_ID=$(sed -n 's/^build_id=//p' <<<"$ids")
VERSION=$(sed -n 's/^version=//p' <<<"$ids")
step "2/6 build ${VERSION} at ${SHA:0:8} (${TARGET}, ${JOBS} jobs, waits for ${LOCK})"
started=$(date +%s)
lockf -k "$LOCK" nice -n 15 env \
  CARGO_BUILD_JOBS="$JOBS" CARGO_INCREMENTAL=0 ZIG="$ZIG" \
  LIBGHOSTTY_VT_OPTIMIZE=ReleaseFast LIBGHOSTTY_VT_SIMD=true \
  HERDR_BUILD_CHANNEL=ac-beta HERDR_BUILD_ID="$BUILD_ID" \
  timeout 2400 rustup run "$TOOLCHAIN" cargo build --release --locked --target "$TARGET"
step "built in $(($(date +%s) - started)) s"

step "3/6 check the binary"
mkdir -p "$STATE/out"
BIN=$STATE/out/$ASSET
cp "target/${TARGET}/release/herdr" "$BIN"
got=$("$BIN" --version)
[ "$got" = "herdr ${VERSION}" ] || die "the binary says '${got}', expected 'herdr ${VERSION}'"
SHA256=$(shasum -a 256 "$BIN" | cut -d' ' -f1)
echo "$got  sha256 ${SHA256}"

if [ -n "$DRY_RUN" ]; then
  step "dry run: nothing published (${BIN})"
  exit 0
fi

step "4/6 publish the beta prerelease"
BETA_NOTES="Rolling \`-ac-beta\` build (\`${VERSION}\`) from \`${REF}\`, built on $(hostname -s) (Apple silicon only)."$'\n\n'"Install: \`brew install colangelo/tap/herdr-beta\`"$'\n'"Upgrade: \`brew update && brew upgrade herdr-beta\`"
GH_TOKEN=$(gh auth token) GITHUB_REPOSITORY=$REPO GITHUB_RUN_ID="local-$(date +%s)" \
  BETA_VERSION=$VERSION TARGET_SHA=$SHA SOURCE_REF=$REF BETA_NOTES=$BETA_NOTES \
  "$SCRIPTS/beta_publish.sh" "$BIN"

step "5/6 update Formula/herdr-beta.rb in ${TAP}"
TAPDIR=$STATE/tap
if [ ! -d "$TAPDIR/.git" ]; then
  git clone --quiet "https://github.com/${TAP}.git" "$TAPDIR"
fi
git -C "$TAPDIR" fetch --quiet origin
git -C "$TAPDIR" checkout --quiet --force -B main origin/main
"$SCRIPTS/beta_formula.sh" "$VERSION" "$SHA256" >"$TAPDIR/Formula/herdr-beta.rb"
git -C "$TAPDIR" add Formula/herdr-beta.rb
if git -C "$TAPDIR" diff --staged --quiet; then
  echo "formula already at ${VERSION}"
else
  git -C "$TAPDIR" commit --quiet -m "Update herdr-beta to ${VERSION}"
  git -C "$TAPDIR" push --quiet origin main
fi

step "6/6 published ${VERSION}"
echo "install: brew update && just brew-upgrade herdr-beta (mbm5 first, then m4m)"
