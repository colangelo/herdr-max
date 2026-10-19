#!/usr/bin/env bash
# Print Formula/herdr-beta.rb for colangelo/homebrew-tap, for both beta
# builders: .github/workflows/beta.yml (both macOS targets) and
# scripts/beta_local.sh (Apple silicon only).
#
#   scripts/beta_formula.sh VERSION ARM64_SHA256 [X86_64_SHA256]
#
# Without an x86_64 checksum the formula is Apple-silicon only: no Intel Mac
# is left in the fleet, and a local beta builds just the arm64 binary
# (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/235).
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 VERSION ARM64_SHA256 [X86_64_SHA256]" >&2
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

cat <<FORMULA
class HerdrBeta < Formula
  desc "Terminal workspace manager for AI coding agents (beta channel)"
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
cat <<'FORMULA'

  def install
    binary = Dir["herdr-*"].first || "herdr"
    bin.install binary => "herdr-beta"
  end

  def caveats
    <<~EOS
      herdr runs a persistent background server. To upgrade without killing
      your running panes, migrate the server onto the new binary after upgrading:

        brew upgrade herdr-beta
        herdr-beta server live-handoff --import-exe "#{opt_bin}/herdr-beta"

      (self-update is disabled for Homebrew installs; use brew upgrade.)
    EOS
  end

  test do
    assert_match "herdr #{version}", shell_output("#{bin}/herdr-beta --version")
  end
end
FORMULA
