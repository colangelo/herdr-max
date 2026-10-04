// Tests for the herdr-attention mod (fork issue 157). `claude plugin test`.
import { expect, mock, test } from 'claude-code/testing'

type Run = { argv: string[] }

const HERDR_ENV = { HERDR_PANE_ID: 'w1:p1', HERDR_SOCKET_PATH: '/tmp/herdr.sock' }

// Stubs shared by the tests: a herdr pane in the environment, a recorder for
// every process the mod starts, a recorder for its log lines, and the engine
// beneath the mod (an optional hold that keeps a tool call open). Every stub is
// registered before the test's first call on $.
function harness(on: any, env: Record<string, string> = HERDR_ENV, exitCode = 0) {
  const clock = mock.clock(on, { now: 1_790_000_000_000 })
  mock.env(on, env)
  const runs: Run[] = []
  const logs: string[] = []
  on('process.run', ($: any, e: any) => {
    runs.push({ argv: e.argv })
    return { value: { exitCode, stdout: '', stderr: exitCode ? 'refused' : '' } }
  })
  on('ui.log', ($: any, e: any) => {
    logs.push(e.text)
    return { value: undefined }
  })
  const engine = { hold: null as Promise<unknown> | null }
  on('session.start', () => ({ cwd: '/work' }))
  on('session.end', () => ({ sessionId: 'test-session' }))
  on('turn.complete', () => ({ text: '' }))
  on('tool.call', async () => {
    if (engine.hold) await engine.hold
    return { result: 'ok' }
  })
  return { clock, runs, logs, engine }
}

// The helper's arguments after `sh <script>`: socket, pane, kind|clear, id, seq, ttl.
const call = (run: Run) => ({
  socket: run.argv[2],
  pane: run.argv[3],
  kind: run.argv[4],
  id: run.argv[5],
  seq: Number(run.argv[6]),
  ttl: run.argv[7],
})
const kinds = (runs: Run[]) => runs.map((run) => call(run).kind)

const startSession = ($: any) => $.session.start({ surface: 'terminal', isInteractive: true, cwd: '/work' })
const endTurn = ($: any, extra: Record<string, unknown> = {}) =>
  $.turn.complete({ turnId: 't1', answer: '', durationMs: 5, isAborted: false, usage: null, ...extra })

// An AskUserQuestion call that stays open until `release` is called.
function openQuestion($: any, engine: { hold: Promise<unknown> | null }, id: string, extra: Record<string, unknown> = {}) {
  let release: (v: unknown) => void = () => {}
  engine.hold = new Promise((resolve) => { release = resolve })
  const pending = $.tool.call({ tool: 'AskUserQuestion', tool_use_id: id, ...extra })
  return { release, pending }
}

test('a question is reported while AskUserQuestion is open and cleared when it settles', async ($, on) => {
  const { runs, clock, engine } = harness(on)
  const { release, pending } = openQuestion($, engine, 'toolu_q1')
  await clock.settle()

  expect(runs.length).toBe(1)
  expect(runs[0].argv[0]).toBe('sh')
  expect(runs[0].argv[1].endsWith('hooks/herdr-hint.sh')).toBe(true)
  expect(call(runs[0])).toMatchObject({ socket: '/tmp/herdr.sock', pane: 'w1:p1', kind: 'question', id: 'toolu_q1', ttl: '15000' })

  release(undefined)
  await pending
  await clock.settle()
  expect(kinds(runs)).toEqual(['question', 'clear'])
})

test('other tool calls report nothing, whether or not they ask for permission', async ($, on) => {
  const { runs, clock } = harness(on)
  await $.tool.call({ tool: 'Bash', tool_use_id: 'toolu_a1' })
  await $.tool.call({ tool: 'Edit', tool_use_id: 'toolu_a2' })
  await clock.settle()
  expect(runs.length).toBe(0)
})

test('a turn or session ending with nothing open starts no process', async ($, on) => {
  const { runs, clock } = harness(on)
  await endTurn($)
  await $.session.end({ reason: 'other' })
  await clock.settle()
  expect(runs.length).toBe(0)
})

test('an interrupted turn and the session end clear an open question', async ($, on) => {
  const { runs, clock, engine } = harness(on)
  openQuestion($, engine, 'toolu_q2')
  await clock.settle()
  await endTurn($, { isAborted: true })
  await clock.settle()
  expect(kinds(runs)).toEqual(['question', 'clear'])

  openQuestion($, engine, 'toolu_q3')
  await clock.settle()
  await $.session.end({ reason: 'clear' })
  await clock.settle()
  expect(kinds(runs)).toEqual(['question', 'clear', 'question', 'clear'])
})

test('a subagent cannot mark a question, and its turn ending clears nothing', async ($, on) => {
  const { runs, clock, engine } = harness(on)
  await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_sq', agentId: 'agent-1' })
  await clock.settle()
  expect(runs.length).toBe(0)

  openQuestion($, engine, 'toolu_q4')
  await clock.settle()
  expect(kinds(runs)).toEqual(['question'])
  await endTurn($, { agentId: 'agent-1' })
  await clock.settle()
  expect(kinds(runs)).toEqual(['question'])
})

test('an open question is repeated every five seconds with a rising seq, and the first seq is the wall clock', async ($, on) => {
  const { runs, clock, engine } = harness(on)
  await startSession($)
  openQuestion($, engine, 'toolu_h1')
  await clock.settle()
  expect(runs.length).toBe(1)
  // Wall-clock microseconds plus a counter: the reload-proof sequence.
  expect(call(runs[0]).seq).toBe(1_790_000_000_000 * 1000 + 1)

  await clock.advance(5000)
  await clock.advance(5000)
  expect(runs.length).toBe(3)
  const seqs = runs.map((run) => call(run).seq)
  expect(seqs[1] > seqs[0] && seqs[2] > seqs[1]).toBe(true)
  expect(runs.every((run) => call(run).kind === 'question' && call(run).id === 'toolu_h1')).toBe(true)
})

test('a second session.start does not stack a second heartbeat timer', async ($, on) => {
  const { runs, clock, engine } = harness(on)
  await startSession($)
  await startSession($)
  await startSession($)
  openQuestion($, engine, 'toolu_h2')
  await clock.settle()
  expect(runs.length).toBe(1)

  // One timer: one repeat per five seconds, not one per session.start.
  await clock.advance(5000)
  expect(runs.length).toBe(2)
  await clock.advance(5000)
  expect(runs.length).toBe(3)
})

test('the heartbeat sends nothing while nothing is open', async ($, on) => {
  const { runs, clock } = harness(on)
  await startSession($)
  await clock.advance(60_000)
  expect(runs.length).toBe(0)
})

test('a question open for thirty minutes is released with one log line', async ($, on) => {
  const { runs, logs, clock, engine } = harness(on)
  await startSession($)
  openQuestion($, engine, 'toolu_long')
  await clock.advance(29 * 60 * 1000)
  expect(logs.length).toBe(0)
  expect(call(runs[runs.length - 1]).kind).toBe('question')

  await clock.advance(2 * 60 * 1000)
  expect(logs.length).toBe(1)
  expect(logs[0]).toContain('30 minutes')
  expect(kinds(runs)[runs.length - 1]).toBe('clear')
  const count = runs.length
  await clock.advance(60_000)
  expect(runs.length).toBe(count)
})

test('outside a herdr pane the mod starts nothing', async ($, on) => {
  const { runs, clock, engine } = harness(on, {})
  openQuestion($, engine, 'toolu_x')
  await clock.settle()
  expect(runs.length).toBe(0)
})

test('a failing report is logged once a minute, not every time', async ($, on) => {
  const { runs, logs, clock, engine } = harness(on, HERDR_ENV, 1)
  await startSession($)
  openQuestion($, engine, 'toolu_f')
  await clock.advance(5000)
  await clock.advance(5000)
  expect(runs.length).toBe(3)
  expect(logs.length).toBe(1)
  expect(logs[0]).toContain('herdr hint failed')

  await clock.advance(60_000)
  expect(logs.length).toBe(2)
})
