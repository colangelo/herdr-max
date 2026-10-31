#!/bin/sh
# Sandbox pilot (planner d5a434c, ask a1030-20; copied from security checks/sandbox-pilot, fork issue 256): append every Bash call that looks like a sandbox block to the session's
# log, with the command and at most two matching lines. Registered for BOTH PostToolUseFailure (non-zero exit) and
# PostToolUse (exit 0): a block inside a command that still exits 0 (a pipe ending in jq, `curl -s | jq`, git's
# credential-store write after a good push) was invisible to the failure hook alone (measured 2026-10-31 09:24Z).
# Never the command's output beyond the matching lines, never a secret: the command is cut at 300 characters and anything
# after a known secret-ish word is masked.
log=${SANDBOX_PILOT_LOG:-$HOME/.local/state/herdr-sandbox/violations.jsonl}
mkdir -p "$(dirname "$log")"
in=$(cat)
# no bare "sandbox": it matched commit messages and docs in a command's output (false hit, 09:25Z). OSStatus -26276 is
# Go's TLS verification failing because trustd is unreachable from the sandbox (bao, gh).
pat='Operation not permitted|not on the allow list|Read-only file system|CONNECT tunnel failed|sandbox_violations|OSStatus -26276'
hit=$(printf '%s' "$in" | jq -r '(.error // "") + "\n" + (.tool_response.stderr // "") + "\n" + (.tool_response.stdout // "")' \
  | grep -E "$pat" | head -2 | cut -c1-200)
[ -n "$hit" ] || exit 0
printf '%s' "$in" | jq -c --arg ts "$(date -u +%FT%TZ)" --arg hit "$hit" '{
  ts: $ts, session: .session_id, cwd: .cwd, event: .hook_event_name,
  command: (.tool_input.command // "" | .[0:300] | gsub("(?i)(?<k>password|token|secret)[=: ]+\\S+"; "\(.k)=…")),
  error: ($hit | split("\n") | join(" | ")),
  real_work: null, note: null
}' >> "$log"
exit 0
