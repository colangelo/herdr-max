#!/usr/bin/env bash
# Print a formula for colangelo/homebrew-tap, for both beta builders:
# .github/workflows/beta.yml (both macOS targets) and scripts/beta_local.sh
# (Apple silicon only).
#
#   scripts/beta_formula.sh [--alias] VERSION ARM64_SHA256 [X86_64_SHA256]
#
# Without --alias: Formula/herdr-max.rb, the real one, installing bin/herdr-max.
# With --alias: Formula/herdr-beta.rb, the transitional one (fork issue 330). It
# depends on herdr-max and installs only a bin/herdr-beta link to its opt path, so the two
# formulae never own the same file in bin: the old herdr-beta keg, which owned
# bin/herdr-beta, is replaced by this one in the same `brew upgrade herdr-beta`
# that installs herdr-max, and the old command keeps working. It is the one
# place the alias lives: drop the formula to drop the alias.
#
# Without an x86_64 checksum the formula is Apple-silicon only: no Intel Mac
# is left in the fleet, and a local beta builds just the arm64 binary
# (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/235).
set -euo pipefail

ALIAS=
if [ "${1:-}" = "--alias" ]; then
  ALIAS=1
  shift
fi
if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 [--alias] VERSION ARM64_SHA256 [X86_64_SHA256]" >&2
  exit 2
fi
VERSION=$1
ARM64=$2
X86_64=${3:-}
for sum in "$ARM64" ${X86_64:+"$X86_64"}; do
  if ! [[ "$sum" =~ ^[0-9a-f]{64}$ ]]; then
    echo "'${sum}' is not a sha256" >&2
    exit 2
  fi
done
BASE_URL="https://github.com/colangelo/herdr-max/releases/download/beta"

if [ -n "$ALIAS" ]; then
  CLASS=HerdrBeta
  DESC="The old herdr-beta command, now a link to herdr-max"
else
  CLASS=HerdrMax
  DESC="Terminal workspace manager for AI coding agents (Herdr Max, beta channel)"
fi

cat <<FORMULA
class ${CLASS} < Formula
  desc "${DESC}"
  homepage "https://github.com/colangelo/herdr-max"
  version "${VERSION}"
  license "AGPL-3.0-or-later"

FORMULA
if [ -n "$X86_64" ]; then
  cat <<FORMULA
  on_macos do
    if Hardware::CPU.arm?
      url "${BASE_URL}/herdr-macos-aarch64"
      sha256 "${ARM64}"
    else
      url "${BASE_URL}/herdr-macos-x86_64"
      sha256 "${X86_64}"
    end
  end
FORMULA
else
  cat <<FORMULA
  depends_on arch: :arm64

  on_macos do
    url "${BASE_URL}/herdr-macos-aarch64"
    sha256 "${ARM64}"
  end
FORMULA
fi
if [ -n "$ALIAS" ]; then
  cat <<'FORMULA'

  depends_on "herdr-max"

  def install
    # An absolute link into opt, not install_symlink: that one resolves opt to the
    # versioned keg, which a later `brew upgrade herdr-max` alone would delete.
    bin.mkpath
    (bin/"herdr-beta").make_symlink Formula["herdr-max"].opt_bin/"herdr-max"
  end

  def caveats
    <<~EOS
      herdr-beta is now herdr-max; this formula only keeps the old command
      working as a link to it. Use herdr-max, and drop this one when nothing
      calls herdr-beta any more:

        brew uninstall herdr-beta
    EOS
  end

  test do
    assert_match "herdr #{version}", shell_output("#{bin}/herdr-beta --version")
  end
end
FORMULA
else
  cat <<'FORMULA'

  def install
    binary = Dir["herdr-*"].first || "herdr"
    bin.install binary => "herdr-max"
  end

  def caveats
    <<~EOS
      herdr runs a persistent background server. To upgrade without killing
      your running panes, migrate the server onto the new binary after upgrading:

        brew upgrade herdr-max
        herdr-max server live-handoff --import-exe "#{opt_bin}/herdr-max"

      (self-update is disabled for Homebrew installs; use brew upgrade.)
      The old herdr-beta command stays as a link while the herdr-beta formula
      is installed.
    EOS
  end

  test do
    assert_match "herdr #{version}", shell_output("#{bin}/herdr-max --version")
  end
end
FORMULA
fi
