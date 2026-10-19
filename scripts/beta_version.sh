#!/usr/bin/env bash
# Print the next beta's build id and version as `key=value` lines, for both
# beta builders: .github/workflows/beta.yml (GitHub) and scripts/beta_local.sh
# (`just beta-local`, a Mac of ours).
#
#   scripts/beta_version.sh [RUN_NUMBER]
#
# Environment: GH_REPO (default colangelo/herdr-max), CODENAME (optional pin).
# Run from the repository root (reads Cargo.toml).
#
# The build number leads the id so `brew upgrade herdr-beta` stays monotonic
# (Homebrew compares the number first). It is the larger of the GitHub run
# number and the published beta's number plus one: a local beta takes the next
# number, and a later GitHub run never reuses it
# (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/235). The
# codename is the surname of a Juventus player from 2012 to today, picked from
# the number, so a build reads "0.7.5-ac-beta.7-pirlo", never a timestamp.
set -euo pipefail

NAMES=(pirlo vidal pogba marchisio tevez llorente chiellini bonucci barzagli lichtsteiner asamoah giovinco vucinic buffon evra dybala higuain mandzukic khedira pjanic cuadrado benatia rugani sturaro alves douglascosta bernardeschi matuidi szczesny ronaldo ramsey rabiot deligt demiral danilo alexsandro kulusevski mckennie morata chiesa bentancur arthur vlahovic locatelli kean zakaria gatti bremer paredes kostic milik fagioli miretti perin thuram koopmeiners cambiaso yildiz weah conceicao kalulu kelly digregorio douglasluiz mbangula)

RUN_NUMBER=${1:-0}
REPO=${GH_REPO:-colangelo/herdr-max}
CODENAME=${CODENAME:-}

case "$RUN_NUMBER" in
  '' | *[!0-9]*)
    echo "run number '${RUN_NUMBER}' is not a number" >&2
    exit 2
    ;;
esac

BASE=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

# The published beta's number, 0 when there is none yet. Any other gh failure
# stops here: reading a network error as "no beta" would reuse a number.
if title=$(gh release view beta --repo "$REPO" --json name --jq .name 2>&1); then
  LAST=$(sed -n 's/.*-ac-beta\.\([0-9][0-9]*\).*/\1/p' <<<"$title")
  if [ -z "$LAST" ]; then
    echo "the beta release title '${title}' has no -ac-beta.N number" >&2
    exit 1
  fi
else
  case "$title" in
    *"release not found"*) LAST=0 ;;
    *)
      echo "could not read the published beta: ${title}" >&2
      exit 1
      ;;
  esac
fi

NUMBER=$((LAST + 1))
if [ "$RUN_NUMBER" -gt "$NUMBER" ]; then
  NUMBER=$RUN_NUMBER
fi

NAME=${NAMES[$((NUMBER % ${#NAMES[@]}))]}
# A pinned codename keeps a run of builds reading the same while the number
# still increments. Checked against the pool: a typo would otherwise mint a
# version token nothing else in the channel shares.
if [ -n "$CODENAME" ]; then
  case " ${NAMES[*]} " in
    *" ${CODENAME} "*) NAME=$CODENAME ;;
    *)
      echo "codename '${CODENAME}' is not in the pool" >&2
      exit 1
      ;;
  esac
fi

BUILD_ID="${NUMBER}-${NAME}"
echo "build_id=${BUILD_ID}"
echo "version=${BASE}-ac-beta.${BUILD_ID}"
