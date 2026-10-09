#!/usr/bin/env bash
# Weekly upstream sync measurement (issue 173). Uses git only: no cargo, no build,
# no checkout, no merge. Safe to run any time and from launchd.
#
#   scripts/sync_measure.sh              # report on stdout
#   scripts/sync_measure.sh --notify     # also write the report file and bell `herdr`
#                                        # (quiet when upstream has nothing new)
#
# Env: HERDR_REPO   repo to measure (default ~/_sync/dev/herdr; any checkout works)
#      SYNC_FORK_REF the fork side (default internal/master)
#      SYNC_STATE_DIR where reports go with --notify (default ~/.local/state/herdr-sync)
set -euo pipefail

repo=${HERDR_REPO:-$HOME/_sync/dev/herdr}
fork=${SYNC_FORK_REF:-internal/master}
state=${SYNC_STATE_DIR:-$HOME/.local/state/herdr-sync}
notify=0
[ "${1:-}" = "--notify" ] && notify=1

g() { git -C "$repo" "$@"; }

# Files the fork changes heavily; an upstream commit touching one needs a human look.
flagged='^(src/client/shell/|src/server/|src/layout\.rs|src/handoff|src/persist/|src/config/|src/pane\.rs|src/pane/|src/app/|src/input/|src/raw_input\.rs|src/protocol/|src/detect/|src/terminal/|AGENTS\.md|justfile|build\.rs|Cargo\.(toml|lock)|\.github/|distribution/)'

g fetch -q upstream
g fetch -q internal

today=$(date +%F)
base=$(g merge-base "$fork" upstream/master)
n=$(g rev-list --count "$fork..upstream/master")
report=$(
  echo "# weekly upstream sync $today: $n commits"
  echo
  echo "fork $fork = $(g rev-parse --short=8 "$fork"), upstream/master = $(g rev-parse --short=8 upstream/master), merge-base = $(g rev-parse --short=8 "$base")"
  echo
  if [ "$n" -gt 0 ]; then
    echo "| commit | date | author | subject | files | flagged files |"
    echo "|---|---|---|---|---|---|"
    g log --reverse --format='%h%x09%ad%x09%an%x09%s' --date=short "$fork..upstream/master" |
      while IFS=$'\t' read -r sha day who subj; do
        files=$(g show --format= --name-only "$sha")
        total=$(printf '%s\n' "$files" | grep -c . || true)
        hot=$(printf '%s\n' "$files" | grep -E "$flagged" | tr '\n' ' ' || true)
        echo "| $sha | $day | $who | ${subj//|/\\|} | $total | ${hot:-none} |"
      done
    echo
    echo "Total: $(g diff --shortstat "$fork...upstream/master")"
    echo
    # Signals that decide merge-in-one-go versus steps.
    echo "Signals:"
    for f in AGENTS.md src/protocol/wire.rs Cargo.toml justfile; do
      c=$(g rev-list --count "$fork..upstream/master" -- "$f")
      echo "- $f: $c upstream commits"
    done
    c=$(g diff --name-status -M "$base" upstream/master | grep -cE '^(R|D)' || true)
    echo "- renamed or deleted paths: $c"
    echo
    if out=$(g merge-tree --write-tree --name-only "$fork" upstream/master 2>&1); then
      echo "Textual merge: clean (git merge-tree). A clean merge can still splice code; read the flagged files."
    else
      echo "Textual merge: CONFLICTS"
      printf '%s\n' "$out" | sed -n '2,40p' | sed 's/^/    /'
    fi
  fi
)

printf '%s\n' "$report"

if [ "$notify" = 1 ]; then
  mkdir -p "$state"
  printf '%s\n' "$report" >"$state/$today.md"
  if [ "$n" -eq 0 ]; then
    echo "no new upstream commits; staying quiet" >&2
    exit 0
  fi
  msg="weekly upstream sync $today: $n new commits. Report: $state/$today.md. Open the sync issue and follow .claude/skills/herdr-sync-upstream/SKILL.md."
  # agent-bell first (--wake); queue mode as the fallback so the report is never lost.
  if ! agent-bell send --from herdr-sync-measure --to herdr --wake "$msg"; then
    agent-bell send --from herdr-sync-measure --to herdr --queue "$msg" || echo "agent-bell failed; report is in $state/$today.md" >&2
  fi
fi
