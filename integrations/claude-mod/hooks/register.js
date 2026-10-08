// herdr-attention: tells herdr when Claude Code waits on the user (fork issues
// 137, 157). Design: openspec/changes/claude-mod-attention.
//
// A mod wraps the tool call as middleware, so it sees a question or a
// permission prompt open, and see it end however it ends. It reports through a
// small helper that writes to herdr's socket (no CLI, no binary name), as a
// hint: herdr decides what a hint means, and falls back to the screen when
// there is none.
//
// - question: `AskUserQuestion` opens (main agent only; a subagent cannot ask
//   in this build). Closed when the call settles or aborts (Esc only aborts).
// - permission prompts are NOT reported (decision on issue 157, from the
//   proof run): the screen already gives the same reason as fast, and a mod
//   cannot see a permission prompt being answered, so a hint cost about 0.7 s
//   of blocked state after every allow. Herdr still accepts `kind: permission`.
//   To bring it back, add a `tool.check` hook that does
//   `const decided = await next(e)`, and when `decided.decision === 'ask'` calls
//   `await openDialog($, e.tool_use_id, 'permission')`, then returns `decided`;
//   the close on settle/abort in `onToolCall` already covers it. Do that if a
//   missed or late permission prompt is ever seen live.
// - turn.complete (also an interrupted turn) and session.end close everything.
// - open dialogs are re-reported every 5 s so a long dialog stays blocked and
//   a dead mod ages out in herdr within the 15 s TTL.
// - a dialog open for 30 minutes is released (a missed close must not hold the
//   pane forever), with one logged line.
// - logs only failures, at most one line a minute.

const HEARTBEAT_MS = 5000
const TTL_MS = 15000
const MAX_OPEN_MS = 30 * 60 * 1000
const FAILURE_LOG_EVERY_MS = 60 * 1000

// dialog id (the tool_use_id) -> { kind, since }, oldest first.
const open = new Map()
// What herdr was last told is open, as 'kind:id', or '' for nothing.
let told = ''
// Reports go out one at a time and in order, so a clear never overtakes the
// open it follows.
let queue = Promise.resolve()
let counter = 0
let lastFailureLog = -Infinity

// Wall-clock microseconds plus a counter for equal milliseconds. Not a counter
// that starts at 1: a plugin reload restarts this module while herdr still
// holds the live hint, and a restarted counter would look stale.
async function nextSeq($) {
  counter += 1
  return (await $.clock.now()) * 1000 + (counter % 1000)
}

async function logFailure($, text) {
  const now = await $.clock.now()
  if (now - lastFailureLog < FAILURE_LOG_EVERY_MS) return
  lastFailureLog = now
  $.ui.log(text)
}

async function send($, kind, id) {
  try {
    const pane = await $.env.get('HERDR_PANE_ID')
    const socket = await $.env.get('HERDR_SOCKET_PATH')
    // Not in a herdr pane: nothing to tell.
    if (!pane || !socket) return
    const seq = await nextSeq($)
    const root = $.plugin.root
    const done = await $.process.run(['sh', root + '/hooks/herdr-hint.sh', socket, pane, kind, id, String(seq), String(TTL_MS)])
    if (done.exitCode !== 0) {
      await logFailure($, 'herdr hint failed (' + done.exitCode + '): ' + String(done.stderr || done.stdout || '').slice(0, 160))
    }
  } catch (err) {
    // herdr is not there, or a host rule refused the call: the dialog still
    // goes on, and nothing here may reject (the queue must keep draining).
    try {
      await logFailure($, 'herdr hint threw: ' + String(err).slice(0, 160))
    } catch {
      // Logging failed too: give up quietly.
    }
  }
}

// Tell herdr what is open now: the oldest open dialog, or that nothing is.
// Does nothing when herdr already knows nothing is open (the isOpen gate that
// keeps every turn end of every session from starting a process).
function sync($) {
  const oldest = open.entries().next().value
  const wanted = oldest ? oldest[1].kind + ':' + oldest[0] : ''
  if (wanted === '' && told === '') return
  told = wanted
  const [kind, id] = oldest ? [oldest[1].kind, oldest[0]] : ['clear', '']
  queue = queue.then(() => send($, kind, id))
}

async function openDialog($, id, kind) {
  if (!id || open.has(id)) return
  open.set(id, { kind, since: await $.clock.now() })
  sync($)
}

function closeDialog($, id) {
  if (!open.delete(id)) return
  sync($)
}

function closeAll($) {
  if (open.size === 0) return
  open.clear()
  sync($)
}

// The heartbeat: repeat what is open so herdr's TTL keeps it, and release a
// dialog that has been open too long.
async function beat($) {
  if (open.size === 0) return
  const now = await $.clock.now()
  for (const [id, entry] of open) {
    if (now - entry.since >= MAX_OPEN_MS) {
      open.delete(id)
      await logFailure($, 'released a ' + entry.kind + ' that stayed open 30 minutes')
    }
  }
  if (open.size === 0) {
    sync($)
    return
  }
  // Same hint again: herdr keeps it alive. Sent even though `told` matches.
  const oldest = open.entries().next().value
  told = oldest[1].kind + ':' + oldest[0]
  queue = queue.then(() => send($, oldest[1].kind, oldest[0]))
}

// One heartbeat timer for the whole module load. `session.start` fires again
// on /clear and /resume, and a timer per start would stack: every open
// question would be sent N times every five seconds.
let heartbeatStarted = false

async function onSessionStart($, e, next) {
  if (!heartbeatStarted) {
    heartbeatStarted = true
    $.clock.every(HEARTBEAT_MS, () => beat($))
  }
  return next(e)
}

async function onToolCall($, e, next) {
  const id = e.tool_use_id
  // A subagent's question must not mark the parent pane.
  if (e.tool === 'AskUserQuestion' && !e.agentId) await openDialog($, id, 'question')
  // Esc abandons the call without settling `next`, so `finally` alone is not
  // enough: the abort signal is the other way a dialog closes.
  const onAbort = () => closeDialog($, id)
  next.signal.addEventListener('abort', onAbort, { once: true })
  try {
    return await next(e)
  } finally {
    next.signal.removeEventListener('abort', onAbort)
    closeDialog($, id)
  }
}

async function onTurnComplete($, e, next) {
  // A subagent's turn ending says nothing about the parent's dialogs.
  if (!e.agentId) closeAll($)
  return next(e)
}

async function onSessionEnd($, e, next) {
  closeAll($)
  return next(e)
}

export function register(on) {
  on('session.start', onSessionStart)
  on('tool.call', onToolCall)
  on('turn.complete', onTurnComplete)
  on('session.end', onSessionEnd)
}
